//! Cloud sessions through the bundled Quick agent.
//!
//! Quick's renderer reaches session storage through an `invokeAgent` qbs-event
//! on the tenant data plane. The request needs an SNCS `id_token`, which only
//! the bundled `quickwork-agent` can mint for an account. This module keeps one
//! account's outpost alive (see `CachedAgent`) and, on first use:
//!
//! 1. copies the account's profile into a temporary base,
//! 2. starts `quickwork-agent --outpost-only` with its own instance token,
//! 3. reads `/auth/sncs-session`,
//! 4. calls `invokeAgent` (`api/sessions`, `store/sessions/<id>/messages`, ...),
//! 5. reuses the agent until the account changes, an account's sign-in or
//!    files change (`invalidate`), or it is idle,
//! 6. stops it with every process it started and removes the temporary base.
//!
//! Listing, previewing and exporting only read. The writes are explicit: moving
//! sessions to another account (new sessions there, the source untouched) and
//! deleting sessions the user picked. Local writes are the temporary copy and
//! the export files.

use super::device;
use crate::error::AppError;
use super::profiles;
use super::switcher;
use base64::Engine;
use reqwest::Client;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

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
    #[error("云端拒绝了登录凭据：{0}")]
    Unauthorized(String),
    #[error("会话 id 不合法：{0}")]
    BadId(String),
    #[error("没有可导出的会话")]
    NothingToExport,
    #[error("没有选择要导入的会话")]
    NothingSelected,
    #[error("会话 {0} 没有可导入的事件")]
    NoEvents(String),
    #[error("没有可上传的事件")]
    NothingUploaded,
    #[error("转入中途失败（已转入 {done} 个）：{detail}")]
    ImportFailed { done: usize, left_behind: Option<String>, detail: String },
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
    /// sessions whose messages may be cut off at the page limit
    pub truncated: Vec<String>,
}

/// Sessions per page, and a guard against a cursor that never ends.
const PAGE_SIZE: &str = "100";
const MAX_PAGES: usize = 500;
/// Messages read per session (that endpoint isn't paged here).
const MESSAGE_LIMIT: usize = 100;

/// Keep one account's outpost alive between requests. Starting the Python
/// backend and copying the profile costs several seconds, so paging through a
/// conversation must not do it per page.
const AGENT_IDLE: Duration = Duration::from_secs(5 * 60);
/// Temporary bases this old are left over from a crash.
const STALE_BASE: Duration = Duration::from_secs(24 * 60 * 60);

struct CachedAgent {
    account_id: String,
    generation: u64,
    /// the agent's own sign-in was refused; start a new one next time
    stale: bool,
    agent: Agent,
    id_token: String,
    tenant: String,
    usable_until: f64,
    last_used: Instant,
}

static AGENT: OnceLock<tokio::sync::Mutex<Option<CachedAgent>>> = OnceLock::new();
static WATCHDOG: AtomicBool = AtomicBool::new(false);
/// Bumped when an account's sign-in or files change; an agent started under
/// an older generation is replaced on its next use.
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// The running agent's pid, so exit can end it without waiting for the lock.
static AGENT_PID: AtomicU32 = AtomicU32::new(0);

fn cache() -> &'static tokio::sync::Mutex<Option<CachedAgent>> {
    AGENT.get_or_init(|| tokio::sync::Mutex::new(None))
}

fn now_secs() -> f64 {
    chrono::Utc::now().timestamp() as f64
}

/// Unix seconds from seconds, milliseconds or an RFC 3339 string; 0 when unknown.
fn as_secs(v: Option<&Value>) -> f64 {
    let n = match v {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => {
            let s = s.trim();
            s.parse::<f64>()
                .ok()
                .or_else(|| chrono::DateTime::parse_from_rfc3339(s).ok().map(|t| t.timestamp_millis() as f64 / 1000.0))
                .unwrap_or(0.0)
        }
        _ => 0.0,
    };
    // no date in seconds is this large; it is milliseconds
    if n > 1e12 { n / 1000.0 } else { n }
}

impl CachedAgent {
    async fn refresh_if_needed(&mut self) -> Result<(), CloudError> {
        if now_secs() + 60.0 < self.usable_until {
            return Ok(());
        }
        self.refresh().await
    }

    async fn refresh(&mut self) -> Result<(), CloudError> {
        let (id_token, tenant, usable_until) = self.agent.sncs().await?;
        self.id_token = id_token;
        self.tenant = tenant;
        self.usable_until = usable_until;
        Ok(())
    }

    /// Run a request with the current token. When the cloud refuses it, get
    /// a new token and try once more; refused again, the agent is replaced on
    /// the next request.
    async fn call<T, F, Fut>(&mut self, f: F) -> Result<T, CloudError>
    where
        F: Fn(String, String) -> Fut,
        Fut: Future<Output = Result<T, CloudError>>,
    {
        match f(self.id_token.clone(), self.tenant.clone()).await {
            Err(CloudError::Unauthorized(_)) => {
                self.refresh().await?;
                let again = f(self.id_token.clone(), self.tenant.clone()).await;
                if matches!(again, Err(CloudError::Unauthorized(_))) {
                    self.stale = true;
                }
                again
            }
            other => other,
        }
    }
}

/// Dropping an agent kills its processes and deletes its files, which blocks;
/// keep that off the async workers.
fn retire<T: Send + 'static>(old: T) {
    tokio::task::spawn_blocking(move || drop(old));
}

/// Remove temporary bases a crashed run left behind.
fn remove_stale_bases() {
    let Ok(entries) = fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with("aqm-cloud-") {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > STALE_BASE);
        if old {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

fn start_watchdog() {
    if WATCHDOG.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::task::spawn_blocking(remove_stale_bases);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let mut slot = cache().lock().await;
            if slot.as_ref().is_some_and(|c| c.last_used.elapsed() >= AGENT_IDLE) {
                retire(slot.take());
            }
        }
    });
}

/// Reuse the account's outpost; start a new one for another account, after
/// `invalidate`, or when the old one is idle or refused.
async fn acquire(account_id: &str) -> Result<tokio::sync::MutexGuard<'static, Option<CachedAgent>>, CloudError> {
    start_watchdog();
    let generation = GENERATION.load(Ordering::SeqCst);
    let mut slot = cache().lock().await;
    let replace = match slot.as_ref() {
        None => true,
        Some(c) => {
            c.account_id != account_id || c.generation != generation || c.stale || c.last_used.elapsed() >= AGENT_IDLE
        }
    };
    if replace {
        retire(slot.take());
        let id = account_id.to_string();
        let agent = tokio::task::spawn_blocking(move || Agent::start(&id))
            .await
            .map_err(|e| CloudError::Spawn(e.to_string()))??;
        let ready = match agent.wait_ready().await {
            Ok(()) => agent.sncs().await,
            Err(e) => Err(e),
        };
        let (id_token, tenant, usable_until) = match ready {
            Ok(v) => v,
            Err(e) => {
                retire(agent);
                return Err(e);
            }
        };
        *slot = Some(CachedAgent {
            account_id: account_id.to_string(),
            generation,
            stale: false,
            agent,
            id_token,
            tenant,
            usable_until,
            last_used: Instant::now(),
        });
    }
    let cached = slot.as_mut().expect("just inserted");
    cached.refresh_if_needed().await?;
    cached.last_used = Instant::now();
    Ok(slot)
}

/// Stop the cached outpost and wait until it is gone.
pub async fn shutdown() {
    let old = cache().lock().await.take();
    if let Some(old) = old {
        let _ = tokio::task::spawn_blocking(move || drop(old)).await;
    }
}

/// An account was switched, signed in again, removed or restored: the cached
/// outpost runs on the old files, so the next request starts a new one.
pub fn invalidate() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// For app exit: never wait for a request that holds the outpost (a transfer
/// can take a minute); end its processes directly instead.
pub fn shutdown_now() {
    if let Some(lock) = AGENT.get()
        && let Ok(mut slot) = lock.try_lock()
    {
        drop(slot.take());
        return;
    }
    let pid = AGENT_PID.swap(0, Ordering::SeqCst);
    if pid != 0 {
        kill_pid_tree(pid);
    }
}

/// `api/sessions` for the first page, `api/sessions/page/<cursor>` after.
fn page_path(cursor: Option<&str>) -> String {
    match cursor {
        None => "api/sessions".into(),
        Some(c) => format!("api/sessions/page/{}", url::form_urlencoded::byte_serialize(c.as_bytes()).collect::<String>()),
    }
}

/// The cursor for the next page; none (null, empty or missing) on the last.
fn next_cursor(body: &Value) -> Option<String> {
    body.get("nextCursor").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

fn session_of(item: &Value) -> Option<CloudSession> {
    let id = item.get("id").and_then(Value::as_str).filter(|s| !s.is_empty())?.to_string();
    Some(CloudSession {
        id,
        title: item.get("title").and_then(Value::as_str).unwrap_or_default().to_string(),
        created_at: item.get("created_at").and_then(Value::as_f64).unwrap_or(0.0),
        updated_at: item.get("updated_at").and_then(Value::as_f64).unwrap_or(0.0),
        message_count: item.get("message_count").and_then(Value::as_i64).unwrap_or(0),
        archived: item.get("archived").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// Every session of the account, following `nextCursor` until it runs out.
/// The API reports neither a total nor a "has more" flag.
async fn list_all(client: &Client, token: &str, tenant: &str) -> Result<Vec<CloudSession>, CloudError> {
    let mut out = Vec::new();
    let mut ids = HashSet::new();
    let mut cursors = HashSet::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let body = invoke(client, token, tenant, &page_path(cursor.as_deref()), json!({ "_method": "GET", "limit": PAGE_SIZE }), json!({})).await?;
        for item in body.get("sessions").and_then(Value::as_array).into_iter().flatten() {
            if let Some(s) = session_of(item)
                && ids.insert(s.id.clone())
            {
                out.push(s);
            }
        }
        match next_cursor(&body) {
            // a cursor seen before would loop forever
            Some(c) if cursors.insert(c.clone()) => cursor = Some(c),
            _ => break,
        }
    }
    Ok(out)
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

/// Folders of a profile the backend doesn't need to sign in. The session
/// store and the knowledge base can be large, and Quick may be writing them;
/// an account with neither (a freshly added one) already lists its cloud
/// sessions fine.
const SKIP_DIRS: &[&str] = &["knowledge_storage", "sessions", "logs", "cache", "Cache", "metrics"];
/// Larger files are databases or caches, not sign-in state.
const MAX_COPY: u64 = 16 * 1024 * 1024;

fn copy_profile(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let name = entry.file_name();
        let to = dst.join(&name);
        let meta = entry.metadata()?;
        if meta.is_dir() {
            if !SKIP_DIRS.iter().any(|s| name.to_string_lossy() == *s) {
                copy_profile(&from, &to)?;
            }
        } else if meta.len() <= MAX_COPY {
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
        let result = Self::spawn_in(root.clone(), &real_base, &source, &agent, entry);
        if result.is_err() {
            let _ = fs::remove_dir_all(&root);
        }
        result
    }

    /// Prepare the temp base under `root` and start the agent there. The
    /// caller removes `root` if this fails.
    fn spawn_in(root: PathBuf, real_base: &Path, source: &Path, agent: &Path, entry: profiles::ProfileEntry) -> Result<Self, CloudError> {
        let base = root.join("base");
        let profile = base.join("profiles").join(&entry.id);
        fs::create_dir_all(&profile)?;
        copy_profile(source, &profile)?;
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
        let mut command = Command::new(agent);
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
        // its own process group, so everything it starts can be ended with it
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn().map_err(|e| CloudError::Spawn(e.to_string()))?;
        #[cfg(windows)]
        job::adopt(&child);
        AGENT_PID.store(child.id(), Ordering::SeqCst);
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
            if let Ok(r) = client.get(&url).header("Authorization", self.auth_header()).send().await
                && r.status().is_success()
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        Err(CloudError::NotReady("health check timed out".into()))
    }

    async fn sncs(&self) -> Result<(String, String, f64), CloudError> {
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
        let usable_until = Some(as_secs(v.get("usable_until"))).filter(|t| *t > 0.0).unwrap_or_else(|| now_secs() + 300.0);
        if id_token.is_empty() || tenant.is_empty() {
            return Err(CloudError::Session("response is missing id_token or tenant_url".into()));
        }
        Ok((id_token, tenant, usable_until))
    }
}

/// End a process and everything it started (the agent launches helpers
/// such as its sandbox). Killing only the agent would leave those running.
fn kill_pid_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(0x0800_0000)
            .output();
    }
    #[cfg(unix)]
    {
        // the agent leads its own process group
        let _ = Command::new("kill").args(["-KILL", "--", &format!("-{pid}")]).output();
    }
}

fn kill_tree(child: &mut Child) {
    kill_pid_tree(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

/// Windows: the agent joins a job that is closed, and its processes ended,
/// when this process ends, so it can't outlive the manager even after a crash,
/// a kill from Task Manager or the updater closing the app.
#[cfg(windows)]
mod job {
    use std::os::windows::io::AsRawHandle;
    use std::sync::OnceLock;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_BASIC_LIMIT_INFORMATION,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    use windows::core::PCWSTR;

    struct Job(HANDLE);
    // a kernel handle, usable from any thread
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    static JOB: OnceLock<Option<Job>> = OnceLock::new();

    fn job() -> Option<&'static Job> {
        JOB.get_or_init(|| {
            let info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
                BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION {
                    LimitFlags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                    ..Default::default()
                },
                ..Default::default()
            };
            // SAFETY: a new unnamed job; `info` outlives the call. The handle is
            // never closed: the system closes it when this process ends.
            unsafe {
                let handle = CreateJobObjectW(None, PCWSTR::null()).ok()?;
                SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    std::ptr::from_ref(&info).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
                .ok()?;
                Some(Job(handle))
            }
        })
        .as_ref()
    }

    pub fn adopt(child: &std::process::Child) {
        if let Some(job) = job() {
            // SAFETY: the child's process handle is open while `child` lives
            unsafe {
                let _ = AssignProcessToJobObject(job.0, HANDLE(child.as_raw_handle()));
            }
        }
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = AGENT_PID.compare_exchange(self.child.id(), 0, Ordering::SeqCst, Ordering::SeqCst);
        kill_tree(&mut self.child);
        // a process that just died can hold its files for a moment on Windows
        for _ in 0..5 {
            if fs::remove_dir_all(&self.base).is_ok() || !self.base.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
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
            let text = format!("HTTP {status}: {}", serde_json::to_string(&body).unwrap_or_default().chars().take(300).collect::<String>());
            return Err(if status == 401 || status == 403 { CloudError::Unauthorized(text) } else { CloudError::Api(text) });
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
        let text = format!("qbs-event HTTP {status}: {}", text.chars().take(300).collect::<String>());
        let refused = status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN;
        return Err(if refused { CloudError::Unauthorized(text) } else { CloudError::Api(text) });
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

#[derive(Debug, Clone, Serialize)]
pub struct CloudSessionPage {
    pub sessions: Vec<CloudSession>,
    /// pass back for the next page; none on the last
    pub next_cursor: Option<String>,
}

/// One page of the account's sessions, newest first, for the list view.
pub async fn list_page(account_id: &str, cursor: Option<String>, limit: usize) -> Result<CloudSessionPage, CloudError> {
    let mut slot = acquire(account_id).await?;
    let cached = slot.as_mut().expect("acquire inserts");
    let client = Client::builder().timeout(Duration::from_secs(60)).build().map_err(|e| CloudError::Api(e.to_string()))?;
    let path = page_path(cursor.as_deref().filter(|c| !c.is_empty()));
    let query = json!({ "_method": "GET", "limit": limit.clamp(1, 100).to_string() });
    let body = cached
        .call(|token, tenant| {
            let (client, path, query) = (&client, path.clone(), query.clone());
            async move { invoke(client, &token, &tenant, &path, query, json!({})).await }
        })
        .await?;
    let sessions = body.get("sessions").and_then(Value::as_array).into_iter().flatten().filter_map(session_of).collect();
    // a cursor that points back at itself would page forever
    let next_cursor = next_cursor(&body).filter(|c| Some(c.as_str()) != cursor.as_deref());
    Ok(CloudSessionPage { sessions, next_cursor })
}

pub async fn list(base: &Path, account_id: &str) -> Result<Vec<CloudSession>, CloudError> {
    let _ = base;
    let mut slot = acquire(account_id).await?;
    let cached = slot.as_mut().expect("acquire inserts");
    let client = Client::builder().timeout(Duration::from_secs(60)).build().map_err(|e| CloudError::Api(e.to_string()))?;
    cached
        .call(|token, tenant| {
            let client = &client;
            async move { list_all(client, &token, &tenant).await }
        })
        .await
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

pub async fn export(base: &Path, account_id: &str, ids: &[String], format: &str, lang: &str) -> Result<CloudExport, CloudError> {
    let _ = base;
    let mut slot = acquire(account_id).await?;
    let cached = slot.as_mut().expect("acquire inserts");
    let client = Client::builder().timeout(Duration::from_secs(90)).build().map_err(|e| CloudError::Api(e.to_string()))?;
    let all = cached
        .call(|token, tenant| {
            let client = &client;
            async move { list_all(client, &token, &tenant).await }
        })
        .await?;
    let id_token = cached.id_token.clone();
    let tenant = cached.tenant.clone();
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
    let mut truncated = Vec::new();
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
        if next_cursor(&body).is_some() || messages.len() >= MESSAGE_LIMIT {
            truncated.push(session.id.clone());
        }
        messages_total += messages.len();
        docs.push((session.id.clone(), json!({ "session": session, "messages": messages })));
    }

    if markdown {
        let l = super::sessions::export_labels(lang);
        let mut text = String::new();
        for (_, doc) in &docs {
            let s = &doc["session"];
            text.push_str(&format!(
                "# {}\n\n- {}: {}\n- {}: {}\n- {}: {}\n\n",
                s.get("title").and_then(Value::as_str).filter(|t| !t.is_empty()).unwrap_or(l.untitled),
                l.session_id,
                s.get("id").and_then(Value::as_str).unwrap_or_default(),
                l.messages,
                s.get("message_count").and_then(Value::as_i64).unwrap_or(0),
                l.updated,
                super::sessions::fmt_secs(s.get("updated_at").and_then(Value::as_f64).unwrap_or(0.0)),
            ));
            for m in doc["messages"].as_array().cloned().unwrap_or_default() {
                let role = m.get("role").and_then(Value::as_str).unwrap_or("message");
                let content = m.get("content").cloned().unwrap_or(Value::Null);
                text.push_str(&format!("**{}**\n\n{}\n\n", role, message_text(&content)));
            }
            text.push_str("\n---\n\n");
        }
        fs::write(dir.join("sessions.md"), text)?;
        Ok(CloudExport { path: dir.join("sessions.md").display().to_string(), sessions: docs.len(), messages: messages_total, truncated })
    } else {
        let count = docs.len();
        let values: Vec<Value> = docs.into_iter().map(|(_, v)| v).collect();
        let path = dir.join("sessions.json");
        fs::write(&path, serde_json::to_string_pretty(&Value::Array(values)).unwrap_or_default())?;
        Ok(CloudExport { path: path.display().to_string(), sessions: count, messages: messages_total, truncated })
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

/// Copy one session's events into a new session on the target. On failure
/// the half-built session is deleted; if that fails too, its id comes back
/// with the error so the user can be told to remove it.
async fn copy_one(client: &Client, token: &str, tenant: &str, source_id: &str, events: Vec<Value>) -> Result<CloudImport, (CloudError, Option<String>)> {
    let target_id = create_session(client, token, tenant).await.map_err(|e| (e, None))?;
    let mut uploaded = 0usize;
    let mut failed: Option<CloudError> = None;
    for event in events {
        let Some(event) = rewrite_event(event, source_id, &target_id) else {
            continue;
        };
        if let Err(e) = upload_event(client, token, tenant, &target_id, &event).await {
            failed = Some(e);
            break;
        }
        uploaded += 1;
    }
    if failed.is_none() && uploaded == 0 {
        failed = Some(CloudError::NothingUploaded);
    }
    if let Some(e) = failed {
        let left = delete_session(client, token, tenant, &target_id).await.err().map(|_| target_id);
        return Err((e, left));
    }
    let messages = message_count(client, token, tenant, &target_id).await.unwrap_or(0);
    Ok(CloudImport { source_session_id: source_id.to_string(), target_session_id: target_id, events: uploaded, messages })
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
    for id in ids {
        check_id(id)?;
    }

    // read the source strictly read-only, then hand the cache to the target
    let mut source_slot = acquire(source_account).await?;
    let source = source_slot.as_mut().expect("acquire inserts");
    let client = Client::builder().timeout(Duration::from_secs(90)).build().map_err(|e| CloudError::Api(e.to_string()))?;
    let mut sources: Vec<(String, Vec<Value>)> = Vec::new();
    for id in ids {
        let events = fetch_events(&client, &source.id_token, &source.tenant, id).await?;
        if events.is_empty() {
            return Err(CloudError::NoEvents(id.clone()));
        }
        sources.push((id.clone(), events));
    }
    drop(source_slot);

    let mut target_slot = acquire(target_account).await?;
    let target = target_slot.as_mut().expect("acquire inserts");
    let mut imports = Vec::new();
    for (source_id, events) in sources {
        match copy_one(&client, &target.id_token, &target.tenant, &source_id, events).await {
            Ok(done) => imports.push(done),
            // nothing moved and nothing left over: the plain error says it all
            Err((e, None)) if imports.is_empty() => return Err(e),
            Err((e, left_behind)) => {
                return Err(CloudError::ImportFailed { done: imports.len(), left_behind, detail: e.to_string() });
            }
        }
    }
    Ok(imports)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_copy_keeps_sign_in_files_and_skips_big_data() {
        let root = std::env::temp_dir().join(format!("aqm-cloudcopy-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let src = root.join("src");
        fs::create_dir_all(src.join("knowledge_storage")).unwrap();
        fs::create_dir_all(src.join("sessions")).unwrap();
        fs::create_dir_all(src.join("agents")).unwrap();
        fs::write(src.join("device_id"), "dev-1").unwrap();
        fs::write(src.join("agents").join("a.json"), "{}").unwrap();
        fs::write(src.join("knowledge_storage").join("knowledge_v1.db"), "x").unwrap();
        fs::write(src.join("sessions").join("sessions.db"), "x").unwrap();
        let big = fs::File::create(src.join("big.db")).unwrap();
        big.set_len(MAX_COPY + 1).unwrap();
        drop(big);

        let dst = root.join("dst");
        copy_profile(&src, &dst).unwrap();
        assert_eq!(fs::read_to_string(dst.join("device_id")).unwrap(), "dev-1");
        assert!(dst.join("agents").join("a.json").is_file());
        assert!(!dst.join("knowledge_storage").exists());
        assert!(!dst.join("sessions").exists());
        assert!(!dst.join("big.db").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn messages_are_flattened_for_the_preview() {
        let m = json!({
            "id": "m1",
            "role": "assistant",
            "content": [{ "type": "text", "text": "hi" }, { "type": "text", "text": "there" }],
            "timestamp": 1_795_500_000_000u64,
            "toolCalls": [{}, {}]
        });
        let out = message_of(&m, "s1");
        assert_eq!((out.id.as_str(), out.content.as_str(), out.timestamp, out.tool_calls), ("m1", "hi\nthere", 1_795_500_000.0, 2));

        let plain = json!({ "role": "user", "content": "same", "timestamp": "2026-11-24T08:00:00Z" });
        let (a, b) = (message_of(&plain, "s1"), message_of(&plain, "s1"));
        assert_eq!(a.id, b.id);
        assert!(a.id.starts_with("s1-") && a.timestamp > 1_700_000_000.0);
        assert_eq!(message_of(&json!({ "role": "user", "content": null, "text": "t" }), "s1").content, "t");
        assert_eq!(as_secs(Some(&json!(1_795_500_000.5))), 1_795_500_000.5);
        assert_eq!(as_secs(None), 0.0);
    }

    #[test]
    fn session_ids_cannot_leave_their_path() {
        for good in ["sess-1a2b_C3", "a:b.c", "0f8fad5b-d9cb-469f-a165-70867728950e"] {
            assert!(check_id(good).is_ok(), "{good}");
        }
        for bad in ["", ".", "..", "a/b", "a\\b", "a?b", "a#b", "a%2Fb", "a b"] {
            assert!(check_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn page_paths_encode_the_cursor() {
        assert_eq!(page_path(None), "api/sessions");
        assert_eq!(page_path(Some("WzE3OTU+/w==")), "api/sessions/page/WzE3OTU%2B%2Fw%3D%3D");
    }

    #[test]
    fn paging_ends_on_a_null_empty_or_missing_cursor() {
        assert_eq!(next_cursor(&json!({ "nextCursor": "abc" })).as_deref(), Some("abc"));
        assert_eq!(next_cursor(&json!({ "nextCursor": null })), None);
        assert_eq!(next_cursor(&json!({ "nextCursor": "" })), None);
        assert_eq!(next_cursor(&json!({ "sessions": [] })), None);
    }

    #[test]
    fn sessions_need_an_id_and_untitled_ones_stay_untitled() {
        assert!(session_of(&json!({ "title": "x" })).is_none());
        let s = session_of(&json!({ "id": "s1", "message_count": 3 })).unwrap();
        assert_eq!((s.id.as_str(), s.title.as_str(), s.message_count), ("s1", "", 3));
    }

    #[test]
    fn events_are_pointed_at_the_new_session() {
        let event = json!({
            "id": "evt-1",
            "server_sequence_number": 7,
            "type": "message_user",
            "session_id": "src",
            "thread_id": "src",
            "message_id": "m-1",
            "data": { "thread_id": "src", "storage_session_id": "src", "session_id": "src", "content": "hi src" }
        });
        let out = rewrite_event(event, "src", "dst").unwrap();
        assert_eq!(out["session_id"], "dst");
        assert_eq!(out["thread_id"], "dst");
        for key in ["thread_id", "storage_session_id", "session_id"] {
            assert_eq!(out["data"][key], "dst", "{key}");
        }
        // ids the server assigns are dropped; everything else is kept as is
        assert!(out.get("id").is_none() && out.get("server_sequence_number").is_none());
        assert_eq!(out["message_id"], "m-1");
        assert_eq!(out["data"]["content"], "hi src");
    }

    #[test]
    fn session_created_and_untyped_events_are_skipped() {
        assert!(rewrite_event(json!({ "type": "session_created", "session_id": "src" }), "src", "dst").is_none());
        assert!(rewrite_event(json!({ "session_id": "src" }), "src", "dst").is_none());
        // ids of some other session are not touched
        let out = rewrite_event(json!({ "type": "x", "session_id": "other" }), "src", "dst").unwrap();
        assert_eq!(out["session_id"], "other");
    }
}

/// A message as the preview shows it: text flattened, time in unix seconds,
/// and always an id.
#[derive(Debug, Clone, Serialize)]
pub struct CloudMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub timestamp: f64,
    pub attachments: usize,
    pub tool_calls: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CloudMessagePage {
    pub messages: Vec<CloudMessage>,
    pub next_token: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CloudDelete {
    pub session_id: String,
    pub success: bool,
    pub error: Option<AppError>,
}

impl CloudDelete {
    fn done(id: &str) -> Self {
        Self { session_id: id.to_string(), success: true, error: None }
    }

    fn failed(id: &str, error: AppError) -> Self {
        Self { session_id: id.to_string(), success: false, error: Some(error) }
    }
}

fn message_of(m: &Value, session_id: &str) -> CloudMessage {
    let role = m.get("role").and_then(Value::as_str).unwrap_or("message").to_string();
    let content = ["content", "full_content", "text"]
        .iter()
        .filter_map(|k| m.get(*k))
        .filter(|v| !v.is_null())
        .map(message_text)
        .find(|s| !s.trim().is_empty())
        .unwrap_or_default();
    let timestamp = as_secs(m.get("timestamp").or_else(|| m.get("created_at")));
    let id = ["id", "message_id"]
        .iter()
        .filter_map(|k| m.get(*k).and_then(Value::as_str))
        .find(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            // stable across pages, so a message seen twice is shown once
            use sha1::Digest;
            let digest = sha1::Sha1::digest(format!("{role}\0{timestamp}\0{content}").as_bytes());
            format!("{session_id}-{}", &hex::encode(digest)[..16])
        });
    let count = |keys: &[&str]| keys.iter().filter_map(|k| m.get(*k).and_then(Value::as_array)).map(Vec::len).max().unwrap_or(0);
    CloudMessage { id, role, content, timestamp, attachments: count(&["attachments"]), tool_calls: count(&["toolCalls", "tool_calls"]) }
}

/// Session ids go into request paths; anything that could step out of
/// `api/sessions/<id>` is refused.
fn check_id(id: &str) -> Result<(), CloudError> {
    let ok = !id.is_empty()
        && id.len() <= 200
        && !id.chars().all(|c| c == '.')
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'));
    if ok { Ok(()) } else { Err(CloudError::BadId(id.chars().take(80).collect())) }
}

/// One page of a conversation. `before` is the previous page's `next_token`.
pub async fn messages(
    _base: &Path,
    account_id: &str,
    session_id: &str,
    before: Option<String>,
    limit: usize,
) -> Result<CloudMessagePage, CloudError> {
    check_id(session_id)?;
    let mut slot = acquire(account_id).await?;
    let cached = slot.as_mut().expect("acquire inserts");
    let client = Client::builder().timeout(Duration::from_secs(45)).build().map_err(|e| CloudError::Api(e.to_string()))?;
    let mut query = json!({ "_method": "GET", "limit": limit.clamp(1, 100).to_string() });
    if let Some(before) = before.filter(|s| !s.is_empty()) {
        query["before"] = Value::String(before);
    }
    let path = format!("store/sessions/{session_id}/messages");
    let body = cached
        .call(|token, tenant| {
            let (client, path, query) = (&client, path.clone(), query.clone());
            async move { invoke(client, &token, &tenant, &path, query, json!({})).await }
        })
        .await?;
    Ok(CloudMessagePage {
        messages: body
            .get("messages")
            .and_then(Value::as_array)
            .map(|items| items.iter().map(|m| message_of(m, session_id)).collect())
            .unwrap_or_default(),
        next_token: body.get("next_token").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string),
        has_more: body.get("has_more").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// Delete cloud sessions. Each session gets its own result. A reply without
/// a verdict is checked against the session list afterwards; nothing is
/// reported deleted that is still there.
pub async fn delete(_base: &Path, account_id: &str, ids: &[String]) -> Result<Vec<CloudDelete>, CloudError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    for id in ids {
        check_id(id)?;
    }
    let mut slot = acquire(account_id).await?;
    let cached = slot.as_mut().expect("acquire inserts");
    let client = Client::builder().timeout(Duration::from_secs(45)).build().map_err(|e| CloudError::Api(e.to_string()))?;
    let mut out = Vec::new();
    let mut unconfirmed = Vec::new();
    for session_id in ids {
        let path = format!("api/sessions/{session_id}");
        let reply = cached
            .call(|token, tenant| {
                let (client, path) = (&client, path.clone());
                async move { invoke(client, &token, &tenant, &path, json!({ "_method": "DELETE" }), json!({})).await }
            })
            .await;
        match reply {
            Ok(body) => match body.get("success").and_then(Value::as_bool) {
                Some(true) => out.push(CloudDelete::done(session_id)),
                Some(false) => {
                    let detail = body.get("error").and_then(Value::as_str).unwrap_or_default();
                    out.push(CloudDelete::failed(
                        session_id,
                        AppError::new("sessions.delete_refused", format!("云端没有删除这个会话：{detail}")).with("detail", detail),
                    ));
                }
                None => {
                    unconfirmed.push(out.len());
                    out.push(CloudDelete::done(session_id));
                }
            },
            Err(e) => out.push(CloudDelete::failed(session_id, AppError::from(e))),
        }
    }
    if !unconfirmed.is_empty() {
        let listed = cached
            .call(|token, tenant| {
                let client = &client;
                async move { list_all(client, &token, &tenant).await }
            })
            .await;
        match listed {
            Ok(list) => {
                let remaining: HashSet<&str> = list.iter().map(|s| s.id.as_str()).collect();
                for i in unconfirmed {
                    if remaining.contains(out[i].session_id.as_str()) {
                        let id = out[i].session_id.clone();
                        out[i] = CloudDelete::failed(&id, AppError::new("sessions.delete_not_done", "删除请求已发送，但会话仍在云端列表里"));
                    }
                }
            }
            Err(e) => {
                let detail = AppError::from(e).message;
                for i in unconfirmed {
                    let id = out[i].session_id.clone();
                    out[i] = CloudDelete::failed(
                        &id,
                        AppError::new("sessions.delete_unconfirmed", format!("删除后没能确认结果，请刷新列表：{detail}"))
                            .with("detail", detail.clone()),
                    );
                }
            }
        }
    }
    Ok(out)
}
