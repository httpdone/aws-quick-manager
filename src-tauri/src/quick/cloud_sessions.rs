//! Cloud session browsing through the bundled Quick agent.
//!
//! Quick's renderer reaches session storage through an `invokeAgent` qbs-event
//! on the tenant data plane. The request needs an SNCS `id_token`, which only
//! the bundled `quickwork-agent` can mint for an account. So for each request
//! this module:
//!
//! 1. copies the account's profile into a temporary base,
//! 2. starts `quickwork-agent --outpost-only` with its own instance token,
//! 3. reads `/auth/sncs-session`,
//! 4. calls `invokeAgent` (`api/sessions`, `store/sessions/<id>/messages`),
//! 5. stops the agent and removes the temporary base.
//!
//! It is read-only against the account's cloud sessions; the only local writes
//! are the temporary copy and the export file.

use super::device;
use super::profiles;
use super::switcher;
use base64::Engine;
use reqwest::Client;
use serde::Serialize;
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum CloudError {
    #[error("没有这个账号：{0}")]
    NoAccount(String),
    #[error("这个账号还没有 Quick 数据目录，先在 Quick 里登录一次")]
    NoProfile,
    #[error("找不到 quickwork-agent，先启动一次 Quick 再试")]
    NoAgent,
    #[error("启动 Quick 后台失败：{0}")]
    Spawn(String),
    #[error("Quick 后台没有就绪：{0}")]
    NotReady(String),
    #[error("读取 Quick 登录会话失败：{0}")]
    Session(String),
    #[error("云端请求失败：{0}")]
    Api(String),
    #[error("没有可导出的会话")]
    NothingToExport,
    #[error("没有选择要导入的会话")]
    NothingSelected,
    #[error("会话 {0} 没有可导入的事件")]
    NoEvents(String),
    #[error("没有可上传的事件")]
    NothingUploaded,
    #[error("写导出文件失败：{0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize)]
pub struct CloudSession {
    pub id: String,
    pub title: String,
    pub created_at: f64,
    pub updated_at: f64,
    pub message_count: i64,
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CloudExport {
    pub path: String,
    pub sessions: usize,
    pub messages: usize,
}

fn token() -> String {
    let bytes: [u8; 32] = rand::random();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn free_port() -> Result<u16, CloudError> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| CloudError::Spawn(e.to_string()))?;
    Ok(listener.local_addr().map_err(|e| CloudError::Spawn(e.to_string()))?.port())
}

fn agent_path() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(appdata) = std::env::var("APPDATA") {
        candidates.push(PathBuf::from(&appdata).join("quickwork").join("agent").join("quickwork-agent.exe"));
    }
    if let Ok(localappdata) = std::env::var("LOCALAPPDATA") {
        candidates.push(PathBuf::from(&localappdata).join("quickwork").join("agent").join("quickwork-agent.exe"));
    }
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join("Library/Application Support/quickwork/agent/quickwork-agent"));
    }
    for dir in super::desktop::install_dirs() {
        candidates.push(dir.join("resources").join("agent").join("quickwork-agent.exe"));
        candidates.push(dir.join("resources").join("agent").join("quickwork-agent"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

struct Agent {
    child: Child,
    port: u16,
    token: String,
    base: PathBuf,
}

impl Agent {
    fn start(account_id: &str) -> Result<Self, CloudError> {
        let real_base = profiles::base_dir();
        let manifest = profiles::read(&real_base).map_err(|e| CloudError::Api(e.to_string()))?;
        let entry = manifest
            .entries
            .iter()
            .find(|e| e.id == account_id)
            .cloned()
            .ok_or_else(|| CloudError::NoAccount(account_id.to_string()))?;
        let source = device::profile_dir(&real_base, &entry);
        if !source.is_dir() {
            return Err(CloudError::NoProfile);
        }
        let agent = agent_path().ok_or(CloudError::NoAgent)?;

        let root = std::env::temp_dir().join(format!("aqm-cloud-{}", token()));
        let base = root.join("base");
        let profile = base.join("profiles").join(&entry.id);
        fs::create_dir_all(&profile)?;
        copy_dir(&source, &profile)?;
        for name in ["device_configuration_id", "device_space_id"] {
            let from = real_base.join(name);
            if from.is_file() {
                fs::copy(&from, base.join(name))?;
            }
        }
        let mut temp_manifest = profiles::Manifest { version: 1, entries: vec![], last_active: Some(entry.id.clone()), extra: Default::default() };
        let mut entry = entry;
        entry.data_path = format!("profiles/{}", entry.id);
        temp_manifest.entries.push(entry);
        profiles::write(&base, &temp_manifest).map_err(|e| CloudError::Api(e.to_string()))?;

        let port = free_port()?;
        let token = token();
        let mut command = Command::new(&agent);
        command
            .args(["--outpost-only", "--manifest", "desktop", "--port", &port.to_string()])
            .env("QUICKWORK_BASE", &base)
            .env("QUICKWORK_ROOT", &profile)
            .env("QUICKWORK_CLOUD_MODE", "1")
            .env("QUICKWORK_OUTPOST_PORT", port.to_string())
            .env("QUICKWORK_BUILD_STAGE", "prod")
            .env("QUICKWORK_PROFILE_STAGE", "prod")
            .env("PYTHONUNBUFFERED", "1")
            .env("PYTHONUTF8", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let mut child = command.spawn().map_err(|e| CloudError::Spawn(e.to_string()))?;
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = writeln!(stdin, "{token}");
        }
        Ok(Agent { child, port, token, base: root })
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.token)
    }

    async fn wait_ready(&self) -> Result<(), CloudError> {
        let client = Client::builder().timeout(Duration::from_secs(5)).build().map_err(|e| CloudError::NotReady(e.to_string()))?;
        let url = format!("http://127.0.0.1:{}/health", self.port);
        for _ in 0..120 {
            if let Ok(r) = client.get(&url).header("Authorization", self.auth_header()).send().await {
                if r.status().is_success() {
                    return Ok(());
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        Err(CloudError::NotReady("health check timed out".into()))
    }

    async fn sncs(&self) -> Result<(String, String), CloudError> {
        let client = Client::builder().timeout(Duration::from_secs(20)).build().map_err(|e| CloudError::Session(e.to_string()))?;
        let url = format!("http://127.0.0.1:{}/auth/sncs-session", self.port);
        let r = client
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| CloudError::Session(e.to_string()))?;
        let status = r.status();
        let text = r.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(CloudError::Session(format!("HTTP {status}: {}", text.chars().take(200).collect::<String>())));
        }
        let v: Value = serde_json::from_str(&text).map_err(|e| CloudError::Session(e.to_string()))?;
        let id_token = v.get("id_token").and_then(Value::as_str).unwrap_or_default().to_string();
        let tenant = v.get("tenant_url").and_then(Value::as_str).unwrap_or_default().trim_end_matches('/').to_string();
        if id_token.is_empty() || tenant.is_empty() {
            return Err(CloudError::Session("response is missing id_token or tenant_url".into()));
        }
        Ok((id_token, tenant))
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn body_of(resp: &Value) -> Result<Value, CloudError> {
    let mut current = resp.clone();
    if let Value::String(text) = &current {
        current = serde_json::from_str(text).map_err(|e| CloudError::Api(e.to_string()))?;
    }
    if let Some(inner) = current.get("eventResponse").and_then(|e| e.get("invokeAgentResponse")) {
        current = inner.clone();
        if let Value::String(text) = &current {
            current = serde_json::from_str(text).map_err(|e| CloudError::Api(e.to_string()))?;
        }
    }
    if current.get("type").and_then(Value::as_str) == Some("api_response") {
        let status = current.get("status").and_then(Value::as_u64).unwrap_or(200);
        let body = current.get("body").cloned().unwrap_or(Value::Null);
        let body = match body {
            Value::String(text) => serde_json::from_str(&text).unwrap_or(Value::String(text)),
            other => other,
        };
        if status >= 400 {
            return Err(CloudError::Api(format!("HTTP {status}: {}", serde_json::to_string(&body).unwrap_or_default().chars().take(300).collect::<String>())));
        }
        return Ok(body);
    }
    Ok(current)
}

async fn invoke(client: &Client, id_token: &str, tenant: &str, path: &str, query: Value, body: Value) -> Result<Value, CloudError> {
    let payload = json!({
        "eventType": "invokeAgent",
        "eventBody": { "invokeAgent": { "path": path, "queryParams": query, "body": body } }
    });
    let url = format!("{tenant}/integration/quick-work/qbs-event");
    let r = client
        .post(&url)
        .bearer_auth(id_token)
        .header("Content-Type", "application/json")
        .header("x-amzn-qs-internal-trace-id", uuid_like())
        .json(&payload)
        .send()
        .await
        .map_err(|e| CloudError::Api(e.to_string()))?;
    let status = r.status();
    let text = r.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(CloudError::Api(format!("qbs-event HTTP {status}: {}", text.chars().take(300).collect::<String>())));
    }
    let value: Value = serde_json::from_str(&text).map_err(|e| CloudError::Api(e.to_string()))?;
    body_of(&value)
}

fn uuid_like() -> String {
    let mut b: [u8; 16] = rand::random();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex::encode(b);
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

pub async fn list(base: &Path, account_id: &str) -> Result<Vec<CloudSession>, CloudError> {
    let _ = base;
    let agent = Agent::start(account_id)?;
    agent.wait_ready().await?;
    let (id_token, tenant) = agent.sncs().await?;
    let client = Client::builder().timeout(Duration::from_secs(60)).build().map_err(|e| CloudError::Api(e.to_string()))?;
    let body = invoke(
        &client,
        &id_token,
        &tenant,
        "api/sessions",
        json!({ "_method": "GET", "limit": "100" }),
        json!({}),
    )
    .await?;
    let items = body.get("sessions").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut out = Vec::new();
    for item in items {
        let id = item.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
        if id.is_empty() {
            continue;
        }
        out.push(CloudSession {
            id,
            title: item.get("title").and_then(Value::as_str).unwrap_or("未命名会话").to_string(),
            created_at: item.get("created_at").and_then(Value::as_f64).unwrap_or(0.0),
            updated_at: item.get("updated_at").and_then(Value::as_f64).unwrap_or(0.0),
            message_count: item.get("message_count").and_then(Value::as_i64).unwrap_or(0),
            archived: item.get("archived").and_then(Value::as_bool).unwrap_or(false),
        });
    }
    Ok(out)
}

fn message_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().map(message_text).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n"),
        Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(Value::as_str) {
                return text.to_string();
            }
            if let Some(text) = map.get("content").and_then(Value::as_str) {
                return text.to_string();
            }
            serde_json::to_string(content).unwrap_or_default()
        }
        _ => content.to_string(),
    }
}

pub async fn export(base: &Path, account_id: &str, ids: &[String], format: &str) -> Result<CloudExport, CloudError> {
    let _ = base;
    let agent = Agent::start(account_id)?;
    agent.wait_ready().await?;
    let (id_token, tenant) = agent.sncs().await?;
    let client = Client::builder().timeout(Duration::from_secs(90)).build().map_err(|e| CloudError::Api(e.to_string()))?;
    let list_body = invoke(
        &client,
        &id_token,
        &tenant,
        "api/sessions",
        json!({ "_method": "GET", "limit": "100" }),
        json!({}),
    )
    .await?;
    let all: Vec<CloudSession> = list_body
        .get("sessions")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let id = item.get("id").and_then(Value::as_str)?.to_string();
                    Some(CloudSession {
                        id,
                        title: item.get("title").and_then(Value::as_str).unwrap_or("未命名会话").to_string(),
                        created_at: item.get("created_at").and_then(Value::as_f64).unwrap_or(0.0),
                        updated_at: item.get("updated_at").and_then(Value::as_f64).unwrap_or(0.0),
                        message_count: item.get("message_count").and_then(Value::as_i64).unwrap_or(0),
                        archived: item.get("archived").and_then(Value::as_bool).unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let selected: Vec<CloudSession> = if ids.is_empty() {
        all
    } else {
        all.into_iter().filter(|s| ids.iter().any(|id| id == &s.id)).collect()
    };
    if selected.is_empty() {
        return Err(CloudError::NothingToExport);
    }

    let markdown = format != "json";
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S").to_string();
    let dir = switcher::app_dir().join("exports").join(&stamp);
    fs::create_dir_all(&dir)?;
    let mut messages_total = 0usize;
    let mut docs: Vec<(String, Value)> = Vec::new();

    for session in &selected {
        let body = invoke(
            &client,
            &id_token,
            &tenant,
            &format!("store/sessions/{}/messages", session.id),
            json!({ "_method": "GET", "limit": "100" }),
            json!({}),
        )
        .await?;
        let messages = body.get("messages").and_then(Value::as_array).cloned().unwrap_or_default();
        messages_total += messages.len();
        docs.push((session.id.clone(), json!({ "session": session, "messages": messages })));
    }

    if markdown {
        let mut text = String::new();
        for (_, doc) in &docs {
            let s = &doc["session"];
            text.push_str(&format!(
                "# {}\n\n- 会话 ID: {}\n- 消息数: {}\n- 更新时间: {}\n\n",
                s.get("title").and_then(Value::as_str).unwrap_or("未命名会话"),
                s.get("id").and_then(Value::as_str).unwrap_or_default(),
                s.get("message_count").and_then(Value::as_i64).unwrap_or(0),
                s.get("updated_at").and_then(Value::as_f64).map(|t| t.to_string()).unwrap_or_else(|| "—".into()),
            ));
            for m in doc["messages"].as_array().cloned().unwrap_or_default() {
                let role = m.get("role").and_then(Value::as_str).unwrap_or("message");
                let content = m.get("content").cloned().unwrap_or(Value::Null);
                text.push_str(&format!("**{}**\n\n{}\n\n", role, message_text(&content)));
            }
            text.push_str("\n---\n\n");
        }
        fs::write(dir.join("sessions.md"), text)?;
        Ok(CloudExport { path: dir.join("sessions.md").display().to_string(), sessions: docs.len(), messages: messages_total })
    } else {
        let count = docs.len();
        let values: Vec<Value> = docs.into_iter().map(|(_, v)| v).collect();
        let path = dir.join("sessions.json");
        fs::write(&path, serde_json::to_string_pretty(&Value::Array(values)).unwrap_or_default())?;
        Ok(CloudExport { path: path.display().to_string(), sessions: count, messages: messages_total })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CloudImport {
    pub source_session_id: String,
    pub target_session_id: String,
    pub events: usize,
    pub messages: usize,
}

fn rewrite_event(mut event: Value, source: &str, target: &str) -> Option<Value> {
    let kind = event.get("type").and_then(Value::as_str).unwrap_or_default().to_string();
    if kind.is_empty() || kind == "session_created" {
        return None;
    }
    if let Some(obj) = event.as_object_mut() {
        if obj.get("session_id").and_then(Value::as_str) == Some(source) {
            obj.insert("session_id".into(), Value::String(target.into()));
        }
        if obj.get("thread_id").and_then(Value::as_str) == Some(source) {
            obj.insert("thread_id".into(), Value::String(target.into()));
        }
        if let Some(data) = obj.get_mut("data").and_then(Value::as_object_mut) {
            for key in ["thread_id", "storage_session_id", "session_id"] {
                if data.get(key).and_then(Value::as_str) == Some(source) {
                    data.insert(key.into(), Value::String(target.into()));
                }
            }
        }
        // the upload endpoint assigns its own id and sequence
        obj.remove("id");
        obj.remove("server_sequence_number");
    }
    Some(event)
}

async fn fetch_events(client: &Client, token: &str, tenant: &str, session_id: &str) -> Result<Vec<Value>, CloudError> {
    let body = invoke(
        client,
        token,
        tenant,
        &format!("store/sessions/{session_id}/events"),
        json!({ "_method": "GET" }),
        json!({}),
    )
    .await?;
    Ok(body
        .as_array()
        .cloned()
        .or_else(|| body.get("events").and_then(Value::as_array).cloned())
        .unwrap_or_default())
}

async fn create_session(client: &Client, token: &str, tenant: &str) -> Result<String, CloudError> {
    let body = invoke(client, token, tenant, "api/sessions", json!({ "_method": "POST" }), json!({})).await?;
    body.get("session_id")
        .or_else(|| body.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| CloudError::Api("creating a session returned no session_id".into()))
}

async fn delete_session(client: &Client, token: &str, tenant: &str, session_id: &str) -> Result<(), CloudError> {
    let _ = invoke(
        client,
        token,
        tenant,
        &format!("api/sessions/{session_id}"),
        json!({ "_method": "DELETE" }),
        json!({}),
    )
    .await?;
    Ok(())
}

async fn upload_event(client: &Client, token: &str, tenant: &str, session_id: &str, event: &Value) -> Result<(), CloudError> {
    let _ = invoke(
        client,
        token,
        tenant,
        &format!("store/sessions/{session_id}/events"),
        json!({ "_method": "POST" }),
        event.clone(),
    )
    .await?;
    Ok(())
}

async fn message_count(client: &Client, token: &str, tenant: &str, session_id: &str) -> Result<usize, CloudError> {
    let body = invoke(
        client,
        token,
        tenant,
        &format!("store/sessions/{session_id}/messages"),
        json!({ "_method": "GET", "limit": "100" }),
        json!({}),
    )
    .await?;
    Ok(body.get("messages").and_then(Value::as_array).map(|a| a.len()).unwrap_or(0))
}

/// Copy sessions from one account to another. The target gets new session ids
/// and new message ids' container; the source sessions are not modified.
pub async fn import_to_account(
    source_account: &str,
    target_account: &str,
    ids: &[String],
) -> Result<Vec<CloudImport>, CloudError> {
    if ids.is_empty() {
        return Err(CloudError::NothingSelected);
    }

    // read the source strictly read-only
    let source = Agent::start(source_account)?;
    source.wait_ready().await?;
    let (source_token, source_tenant) = source.sncs().await?;
    let client = Client::builder().timeout(Duration::from_secs(90)).build().map_err(|e| CloudError::Api(e.to_string()))?;
    let mut sources: Vec<(String, Vec<Value>)> = Vec::new();
    for id in ids {
        let events = fetch_events(&client, &source_token, &source_tenant, id).await?;
        if events.is_empty() {
            return Err(CloudError::NoEvents(id.clone()));
        }
        sources.push((id.clone(), events));
    }
    drop(source);

    let target = Agent::start(target_account)?;
    target.wait_ready().await?;
    let (target_token, target_tenant) = target.sncs().await?;
    let mut imports = Vec::new();
    for (source_id, events) in sources {
        let target_id = create_session(&client, &target_token, &target_tenant).await?;
        let mut uploaded = 0usize;
        let mut failed: Option<CloudError> = None;
        for event in events {
            let Some(event) = rewrite_event(event, &source_id, &target_id) else {
                continue;
            };
            match upload_event(&client, &target_token, &target_tenant, &target_id, &event).await {
                Ok(()) => uploaded += 1,
                Err(e) => {
                    failed = Some(e);
                    break;
                }
            }
        }
        if let Some(e) = failed {
            let _ = delete_session(&client, &target_token, &target_tenant, &target_id).await;
            return Err(e);
        }
        if uploaded == 0 {
            let _ = delete_session(&client, &target_token, &target_tenant, &target_id).await;
            return Err(CloudError::NothingUploaded);
        }
        let messages = message_count(&client, &target_token, &target_tenant, &target_id).await.unwrap_or(0);
        imports.push(CloudImport {
            source_session_id: source_id,
            target_session_id: target_id,
            events: uploaded,
            messages,
        });
    }
    Ok(imports)
}
