//! Add an account through the desktop app's own browser sign-in.
//!
//! The flow is the one magpie-aws-quick reproduced from the app:
//!
//!   https://quick.aws.com/sn?qwSurface=desktop:<nonce>:<port>:<version>
//!     → Amazon resolves the account and redirects to 127.0.0.1:<port>/callback
//!     → dynamic OIDC client registration + PKCE authorize redirect
//!     → 127.0.0.1:<port>/?code&state
//!     → token exchange + user-info
//!
//! Tokens never touch the frontend. They are written, in the desktop's own
//! credential format, to the account vault and to the desktop's credential
//! store, together with a manifest entry and a fresh profile `device_id`.

use super::api;
use super::credstore::{self, RawStore};
use super::device;
use super::profiles;
use super::vault;
use base64::Engine;
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

const DISCOVERY_URL: &str = "https://quick.aws.com/sn";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, thiserror::Error)]
pub enum LoginError {
    #[error("本机回调端口监听失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("打开浏览器失败：{0}")]
    Browser(String),
    #[error("登录超时（10 分钟），请重试")]
    Timeout,
    #[error("{0}")]
    Failed(String),
    #[error("这个账号已经添加过：{0}")]
    Exists(String),
    #[error("先生成登录地址")]
    NoSession,
    #[error("登录会话已过期，请重新生成登录地址")]
    SessionExpired,
    #[error("这不是登录完成后的地址，请粘贴浏览器地址栏里的完整地址")]
    BadPaste,
    #[error(transparent)]
    Vault(#[from] vault::VaultError),
}

/// What the UI gets after a login. No tokens.
#[derive(Debug, Clone, Serialize)]
pub struct LoginOutcome {
    pub id: String,
    pub email: String,
    pub display_name: String,
}

struct Reply {
    status: u16,
    location: Option<String>,
    body: String,
}

struct Callback {
    query: Vec<(String, String)>,
    reply: mpsc::Sender<Reply>,
}

struct StopOnDrop(Arc<AtomicBool>);
impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

fn token() -> String {
    let bytes: [u8; 32] = rand::random();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn challenge(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        302 => "Found",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        504 => "Gateway Timeout",
        _ => "OK",
    }
}

fn html(status: u16, message: &str) -> Reply {
    Reply {
        status,
        location: None,
        body: format!(
            "<!doctype html><meta charset=\"utf-8\"><title>Amazon Quick</title>\
             <body style=\"font-family:system-ui;margin:3rem auto;max-width:34rem;line-height:1.6\">\
             <h2>Amazon Quick</h2><p>{message}</p></body>"
        ),
    }
}

/// Local callback server. Returns the chosen port, a sender for feeding
/// pasted callbacks into the same handler, the receiver, and a stop flag.
fn start_server() -> std::io::Result<(
    u16,
    tokio::sync::mpsc::UnboundedSender<Callback>,
    tokio::sync::mpsc::UnboundedReceiver<Callback>,
    StopOnDrop,
)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    listener.set_nonblocking(true)?;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let tx_out = tx.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::spawn(move || {
        while !flag.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    let mut buf = [0u8; 8192];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]);
                    let target = head
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap_or("/");
                    let query: Vec<(String, String)> = url::Url::parse(&format!("http://127.0.0.1:{port}{target}"))
                        .map(|u| u.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect::<Vec<(String, String)>>())
                        .unwrap_or_default();
                    let (rtx, rrx) = mpsc::channel();
                    if tx.send(Callback { query, reply: rtx }).is_err() {
                        break;
                    }
                    let reply = rrx.recv_timeout(Duration::from_secs(300)).unwrap_or_else(|_| {
                        html(504, "This sign-in took too long. Close this page and start again from the app.")
                    });
                    let mut resp = format!(
                        "HTTP/1.1 {} {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n",
                        reply.status,
                        reason(reply.status),
                        reply.body.len()
                    );
                    if let Some(location) = reply.location {
                        resp.push_str(&format!("Location: {location}\r\n"));
                    }
                    resp.push_str("\r\n");
                    resp.push_str(&reply.body);
                    let _ = stream.write_all(resp.as_bytes());
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(80)),
                Err(_) => break,
            }
        }
    });
    Ok((port, tx_out, rx, StopOnDrop(stop)))
}

fn open_browser(url: &str) -> Result<(), LoginError> {
    #[cfg(windows)]
    let spawned = std::process::Command::new("cmd").args(["/c", "start", "", url]).spawn();
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("open").arg(url).spawn();
    #[cfg(not(any(windows, target_os = "macos")))]
    let spawned: Result<std::process::Child, std::io::Error> = Err(std::io::Error::other("unsupported platform"));
    spawned.map(|_| ()).map_err(|e| LoginError::Browser(e.to_string()))
}

fn field(q: &[(String, String)], keys: &[&str]) -> Option<String> {
    q.iter().find(|(k, _)| keys.contains(&k.as_str())).map(|(_, v)| v.clone())
}

fn has(q: &[(String, String)], key: &str) -> bool {
    q.iter().any(|(k, _)| k.as_str() == key)
}

fn discovery_map(q: &[(String, String)]) -> Map<String, Value> {
    q.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect()
}

/// Query parameters from an address the user pasted. Accepts a full URL, a
/// scheme-less host/path, or a bare query string.
fn parse_pasted_query(pasted: &str) -> Option<Vec<(String, String)>> {
    let text = pasted.trim();
    if text.is_empty() {
        return None;
    }
    let text = text.split('#').next().unwrap_or(text);
    let query = if let Some(i) = text.find('?') {
        &text[i + 1..]
    } else if text.contains('=') && !text.chars().any(char::is_whitespace) {
        text
    } else {
        return None;
    };
    Some(url::form_urlencoded::parse(query.as_bytes()).map(|(k, v)| (k.into_owned(), v.into_owned())).collect())
}

/// A finished login, before the desktop is touched.
pub struct NewAccount {
    id: String,
    email: String,
    display_name: String,
    blob: String,
}

async fn complete_login(
    http: &reqwest::Client,
    query: &[(String, String)],
    client: api::Client,
    discovery: &Map<String, Value>,
    tenant_url: &str,
    redirect_uri: &str,
    verifier: &str,
    state: &str,
) -> Result<NewAccount, LoginError> {
    if field(query, &["state"]).as_deref() != Some(state) {
        return Err(LoginError::Failed("OAuth state 不匹配，请重试".into()));
    }
    if let Some(err) = field(query, &["error"]) {
        let msg = format!("{err} {}", field(query, &["error_description"]).unwrap_or_default()).trim().to_string();
        return Err(LoginError::Failed(msg));
    }
    let code = field(query, &["code"]).ok_or_else(|| LoginError::Failed("回调里没有 code".into()))?;
    let region = discovery
        .get("idcRegion")
        .or_else(|| discovery.get("idc_region"))
        .and_then(Value::as_str)
        .unwrap_or("us-east-1");
    let tokens = api::exchange_code(http, region, &client, &code, redirect_uri, verifier)
        .await
        .map_err(|e| LoginError::Failed(format!("换取令牌失败：{e}")))?;
    if tokens.refresh_token.is_empty() {
        return Err(LoginError::Failed("令牌响应里没有 refresh_token".into()));
    }
    let info = api::user_info(http, tenant_url, &tokens.access_token)
        .await
        .map_err(|e| LoginError::Failed(format!("读取账号信息失败：{e}")))?;
    if info.external_id.is_empty() {
        return Err(LoginError::Failed("账号信息里没有 external_id，无法生成账号 id".into()));
    }
    if info.email.is_empty() && info.display_name.is_empty() {
        return Err(LoginError::Failed("账号信息里没有邮箱或显示名".into()));
    }

    let issuer = discovery
        .get("issuerUrl")
        .or_else(|| discovery.get("issuer_url"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut display = if info.display_name.is_empty() {
        format!("{} {}", info.first_name, info.last_name).trim().to_string()
    } else {
        info.display_name.clone()
    };
    if display.is_empty() {
        display = if info.email.is_empty() { info.external_id.clone() } else { info.email.clone() };
    }
    let now = chrono::Utc::now().timestamp() as f64;
    let lifetime = if tokens.expires_in > 0 { tokens.expires_in } else { 3600 };

    let mut blob = Map::new();
    blob.insert("refresh_token".into(), Value::String(tokens.refresh_token));
    blob.insert("refresh_token_expiry".into(), Value::from(client.client_secret_expires_at as f64));
    blob.insert("access_token".into(), Value::String(tokens.access_token));
    blob.insert("access_token_expiry".into(), Value::from(now + lifetime as f64));
    blob.insert("client_id".into(), Value::String(client.client_id.clone()));
    blob.insert("client_secret".into(), Value::String(client.client_secret));
    blob.insert("client_secret_expires_at".into(), Value::from(client.client_secret_expires_at));
    blob.insert("issuer_url".into(), Value::String(issuer));
    blob.insert("tenant_url".into(), Value::String(tenant_url.to_string()));
    blob.insert("aws_account_id".into(), Value::String(info.aws_account_id));
    blob.insert("account_ownership".into(), Value::String(info.account_ownership));
    blob.insert("account_ownership_checked_at".into(), Value::from(now));
    blob.insert("display_name".into(), Value::String(display.clone()));
    blob.insert("email".into(), Value::String(info.email.clone()));
    blob.insert("first_name".into(), Value::String(info.first_name));
    blob.insert("last_name".into(), Value::String(info.last_name));
    blob.insert("role".into(), Value::String(info.role));
    blob.insert("external_id".into(), Value::String(info.external_id.clone()));

    Ok(NewAccount {
        id: profiles::profile_id("social", &info.external_id),
        email: info.email,
        display_name: display,
        blob: Value::Object(blob).to_string(),
    })
}

/// A manual login that is waiting for the next pasted address.
struct ManualSession {
    nonce: String,
    state: String,
    verifier: String,
    redirect_uri: String,
    started: Instant,
    discovery: Option<Map<String, Value>>,
    client: Option<api::Client>,
    tenant_url: String,
    step: String,
    authorize_url: Option<String>,
    account: Option<LoginOutcome>,
    error: Option<String>,
}

struct ManualHandle {
    state: Arc<tokio::sync::Mutex<ManualSession>>,
    tx: tokio::sync::mpsc::UnboundedSender<Callback>,
    _stop: StopOnDrop,
}

static MANUAL: Mutex<Option<ManualHandle>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize)]
pub struct ManualStatus {
    pub step: String,
    pub url: Option<String>,
    pub account: Option<LoginOutcome>,
    pub error: Option<String>,
}

fn manual_state() -> Result<Arc<tokio::sync::Mutex<ManualSession>>, LoginError> {
    MANUAL
        .lock()
        .map_err(|_| LoginError::Failed("登录状态损坏，请重开应用".into()))?
        .as_ref()
        .map(|h| h.state.clone())
        .ok_or(LoginError::NoSession)
}

/// Start a manual login with a local callback server. If the browser can
/// reach 127.0.0.1 the server finishes both hops without paste; if it cannot,
/// the user pastes each callback and the same handler advances the session.
pub async fn start_manual() -> Result<String, LoginError> {
    let (port, tx, mut rx, stop) = start_server()?;
    let nonce = token();
    let state = token();
    let verifier = token();
    let redirect_uri = format!("http://127.0.0.1:{port}");
    let session = ManualSession {
        nonce: nonce.clone(),
        state,
        verifier,
        redirect_uri,
        started: Instant::now(),
        discovery: None,
        client: None,
        tenant_url: String::new(),
        step: "pending".into(),
        authorize_url: None,
        account: None,
        error: None,
    };
    let shared = Arc::new(tokio::sync::Mutex::new(session));
    let handler_state = shared.clone();
    let base = profiles::base_dir();
    let vault_dir = vault::dir();
    tokio::spawn(async move {
        while let Some(callback) = rx.recv().await {
            handle_manual(callback, handler_state.clone(), &base, &vault_dir).await;
        }
    });
    *MANUAL
        .lock()
        .map_err(|_| LoginError::Failed("登录状态损坏，请重开应用".into()))? = Some(ManualHandle {
        state: shared,
        tx,
        _stop: stop,
    });
    let surface = format!("desktop:{nonce}:{port}:{}", api::APP_VERSION);
    Ok(format!("{DISCOVERY_URL}?qwSurface={surface}"))
}

pub async fn manual_status() -> Result<ManualStatus, LoginError> {
    let state = manual_state()?;
    let mut session = state.lock().await;
    if session.step != "done" && session.step != "failed" && session.started.elapsed() > LOGIN_TIMEOUT {
        session.step = "failed".into();
        session.error = Some("登录超时（10 分钟），请重新生成登录地址".into());
    }
    Ok(ManualStatus {
        step: session.step.clone(),
        url: session.authorize_url.clone(),
        account: session.account.clone(),
        error: session.error.clone(),
    })
}

/// Feed a pasted callback address into the same handler the local server uses.
pub fn submit_manual(pasted: &str) -> Result<(), LoginError> {
    let query = parse_pasted_query(pasted).ok_or(LoginError::BadPaste)?;
    let tx = MANUAL
        .lock()
        .map_err(|_| LoginError::Failed("登录状态损坏，请重开应用".into()))?
        .as_ref()
        .map(|h| h.tx.clone())
        .ok_or(LoginError::NoSession)?;
    let (reply, _drop) = mpsc::channel();
    tx.send(Callback { query, reply }).map_err(|_| LoginError::Failed("登录会话已结束".into()))?;
    Ok(())
}

async fn commit_account(base: &Path, vault_dir: &Path, account: NewAccount) -> Result<LoginOutcome, LoginError> {
    let base = base.to_path_buf();
    let vault_dir = vault_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let store = credstore::system();
        commit(&base, store.as_ref(), &vault_dir, account)
    })
    .await
    .map_err(|e| LoginError::Failed(e.to_string()))?
}

async fn handle_manual(callback: Callback, state: Arc<tokio::sync::Mutex<ManualSession>>, base: &Path, vault_dir: &Path) {
    match advance_manual(&callback.query, &state, base, vault_dir).await {
        Ok(reply) => {
            let _ = callback.reply.send(reply);
        }
        Err(e) => {
            let message = e.to_string();
            {
                let mut session = state.lock().await;
                session.step = "failed".into();
                session.error = Some(message.clone());
            }
            let _ = callback.reply.send(html(400, &message));
        }
    }
}

async fn advance_manual(
    query: &[(String, String)],
    state: &Arc<tokio::sync::Mutex<ManualSession>>,
    base: &Path,
    vault_dir: &Path,
) -> Result<Reply, LoginError> {
    if has(query, "nonce") {
        let (expected, redirect_uri, oauth_state, verifier) = {
            let session = state.lock().await;
            if session.started.elapsed() > LOGIN_TIMEOUT {
                return Err(LoginError::SessionExpired);
            }
            (
                session.nonce.clone(),
                session.redirect_uri.clone(),
                session.state.clone(),
                session.verifier.clone(),
            )
        };
        if field(query, &["nonce"]).as_deref() != Some(expected.as_str()) {
            return Err(LoginError::BadPaste);
        }
        if let Some(err) = field(query, &["error"]) {
            let msg = format!("{err} {}", field(query, &["error_description"]).unwrap_or_default()).trim().to_string();
            return Err(LoginError::Failed(msg));
        }
        let issuer = field(query, &["issuerUrl", "issuer_url"]).unwrap_or_default();
        if issuer.is_empty() {
            return Err(LoginError::Failed("地址里没有 issuerUrl".into()));
        }
        let tenant_url = field(query, &["tenantUrl", "tenant_url"]).unwrap_or_default();
        let region = field(query, &["idcRegion", "idc_region"]).unwrap_or_else(|| "us-east-1".into());
        let registered = api::register_client(&api::client(), &region, &issuer, &redirect_uri)
            .await
            .map_err(|e| LoginError::Failed(format!("注册 OIDC 客户端失败：{e}")))?;
        let authorize = api::authorize_url(&region, &registered.client_id, &redirect_uri, &oauth_state, &challenge(&verifier));
        {
            let mut session = state.lock().await;
            session.discovery = Some(discovery_map(query));
            session.tenant_url = tenant_url;
            session.client = Some(registered);
            session.authorize_url = Some(authorize.clone());
            session.step = "authorize".into();
        }
        return Ok(Reply { status: 302, location: Some(authorize), body: String::new() });
    }

    if has(query, "code") || has(query, "error") {
        let (oauth_state, verifier, redirect_uri, client, discovery, tenant_url) = {
            let mut session = state.lock().await;
            if session.started.elapsed() > LOGIN_TIMEOUT {
                return Err(LoginError::SessionExpired);
            }
            let client = session
                .client
                .take()
                .ok_or_else(|| LoginError::Failed("还没有授权地址，请先完成第一步".into()))?;
            let discovery = session
                .discovery
                .clone()
                .ok_or_else(|| LoginError::Failed("缺少身份中心信息，请重新生成登录地址".into()))?;
            (
                session.state.clone(),
                session.verifier.clone(),
                session.redirect_uri.clone(),
                client,
                discovery,
                session.tenant_url.clone(),
            )
        };
        let account = complete_login(
            &api::client(),
            query,
            client,
            &discovery,
            &tenant_url,
            &redirect_uri,
            &verifier,
            &oauth_state,
        )
        .await?;
        let outcome = commit_account(base, vault_dir, account).await?;
        {
            let mut session = state.lock().await;
            session.step = "done".into();
            session.account = Some(outcome);
            session.error = None;
        }
        return Ok(html(200, "Login complete. Return to AWS Quick Manager; this window can be closed."));
    }

    Err(LoginError::BadPaste)
}

/// Run the browser login. Resolves only after the browser has completed the
/// flow; the desktop is not touched here.
pub async fn login() -> Result<NewAccount, LoginError> {
    let http = api::client();
    let state = token();
    let nonce = token();
    let verifier = token();
    let computed = challenge(&verifier);
    let (port, _tx, mut rx, _stop) = start_server()?;
    let redirect_uri = format!("http://127.0.0.1:{port}");
    let surface = format!("desktop:{nonce}:{port}:{}", api::APP_VERSION);
    open_browser(&format!("{DISCOVERY_URL}?qwSurface={surface}"))?;

    let deadline = tokio::time::Instant::now() + LOGIN_TIMEOUT;
    let mut discovery: Option<Map<String, Value>> = None;
    let mut client: Option<api::Client> = None;
    let mut tenant_url = String::new();

    loop {
        let callback = match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some(c)) => c,
            Ok(None) => return Err(LoginError::Failed("回调服务已停止".into())),
            Err(_) => return Err(LoginError::Timeout),
        };

        if has(&callback.query, "nonce") {
            // hop 1: Amazon resolved the account; register a client and send
            // the browser on to the OIDC authorize page.
            if field(&callback.query, &["nonce"]).as_deref() != Some(nonce.as_str()) {
                let _ = callback.reply.send(html(400, "This sign-in link is stale. Start the login again from the app."));
                return Err(LoginError::Failed("登录链接已过期，请重试".into()));
            }
            if let Some(err) = field(&callback.query, &["error"]) {
                let msg = format!("{err} {}", field(&callback.query, &["error_description"]).unwrap_or_default()).trim().to_string();
                let _ = callback.reply.send(html(400, "Amazon did not complete the sign-in."));
                return Err(LoginError::Failed(msg));
            }
            let issuer = field(&callback.query, &["issuerUrl", "issuer_url"]).unwrap_or_default();
            let tenant = field(&callback.query, &["tenantUrl", "tenant_url"]).unwrap_or_default();
            let region = field(&callback.query, &["idcRegion", "idc_region"]).unwrap_or_else(|| "us-east-1".into());
            if issuer.is_empty() {
                let _ = callback.reply.send(html(400, "Amazon did not return an identity center."));
                return Err(LoginError::Failed("登录回调里没有 issuerUrl".into()));
            }
            let registered = match api::register_client(&http, &region, &issuer, &redirect_uri).await {
                Ok(c) => c,
                Err(e) => {
                    let _ = callback.reply.send(html(500, "Could not register the sign-in client."));
                    return Err(LoginError::Failed(format!("注册 OIDC 客户端失败：{e}")));
                }
            };
            let authorize = api::authorize_url(&region, &registered.client_id, &redirect_uri, &state, &computed);
            discovery = Some(discovery_map(&callback.query));
            tenant_url = tenant;
            client = Some(registered);
            let _ = callback.reply.send(Reply { status: 302, location: Some(authorize), body: String::new() });
            continue;
        }

        if has(&callback.query, "code") || has(&callback.query, "error") {
            let reply = callback.reply;
            let (Some(client), Some(discovery)) = (client.take(), discovery.as_ref()) else {
                let _ = reply.send(html(400, "This sign-in is out of order. Start again from the app."));
                return Err(LoginError::Failed("登录流程顺序不对，请重试".into()));
            };
            let result = complete_login(&http, &callback.query, client, discovery, &tenant_url, &redirect_uri, &verifier, &state).await;
            let _ = reply.send(match &result {
                Ok(_) => html(200, "Login complete. Return to AWS Quick Manager; this window can be closed."),
                Err(e) => html(400, &format!("Sign-in failed: {e}")),
            });
            return result;
        }

        let _ = callback.reply.send(html(200, "Finish the Amazon sign-in in the tab that just opened. This page only confirms the result."));
    }
}

/// Save the new account in the vault only. The desktop is not touched:
/// switching (or the next desktop sync) is what writes credentials and a
/// manifest entry, the same way Antigravity keeps accounts in its own store
/// and only injects them into the IDE on switch.
pub fn commit(base: &Path, store: &dyn RawStore, vault_dir: &Path, account: NewAccount) -> Result<LoginOutcome, LoginError> {
    if profiles::read(base)?.entries.iter().any(|e| e.id == account.id)
        || vault::list(vault_dir).unwrap_or_default().iter().any(|e| e.id == account.id)
    {
        return Err(LoginError::Exists(account.email));
    }

    // one complete identity per account: the profile device id plus the
    // install ids this account projects into the base files on switch
    let device_id = device::new_id();
    let config = device::new_id();
    let space = device::space_id_for(&config);
    vault::upsert(vault_dir, store, &account.id, &account.blob)?;
    vault::set_identity(vault_dir, &account.id, &device_id, &config, &space)?;

    Ok(LoginOutcome { id: account.id, email: account.email, display_name: account.display_name })
}
