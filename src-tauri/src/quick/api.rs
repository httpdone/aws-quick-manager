//! Amazon Quick / IAM Identity Center HTTP calls: client registration, token
//! exchange and refresh, user-info and the agent-hour entitlement.
//!
//! Same endpoints and bodies the desktop app uses (verified against a live
//! account through the magpie-aws-quick plugin).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

pub const SCOPE: &str = "quicksight:read_write";
pub const CLIENT_NAME: &str = "AmazonQuick-Desktop";
pub const DISCOVERY_URL: &str = "https://quick.aws.com/sn";
pub const SOCIAL_DATA_PLANE: &str = "https://cell001.dp.appintegrations.us-east-1.prod.plato.ai.aws.dev";
/// The desktop version the discovery page is told it talks to.
pub const APP_VERSION: &str = "1.2.4660";

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    Http(String),
    /// the sign-in is no longer valid (401/403, invalid_grant)
    #[error("登录已失效：{0}")]
    Auth(String),
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("aws-quick-manager/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("http client")
}

async fn post_json(http: &reqwest::Client, url: &str, body: &Value, bearer: Option<&str>) -> Result<Value, ApiError> {
    let mut req = http.post(url).header("Accept", "application/json").json(body);
    if let Some(t) = bearer {
        req = req.bearer_auth(t);
    }
    let res = req.send().await.map_err(|e| ApiError::Http(format!("{}: {e}", host(url))))?;
    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    if status.as_u16() == 401 || status.as_u16() == 403 || text.contains("invalid_grant") {
        return Err(ApiError::Auth(format!("HTTP {status} from {}", host(url))));
    }
    if !status.is_success() {
        return Err(ApiError::Http(format!("HTTP {status} from {}: {}", host(url), truncate(&text, 300))));
    }
    serde_json::from_str(&text).map_err(|e| ApiError::Http(format!("invalid JSON from {}: {e}", host(url))))
}

fn host(url: &str) -> String {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default()
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn s(v: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|k| v.get(*k).and_then(|x| match x {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }))
        .unwrap_or_default()
}

fn n(v: &Value, keys: &[&str]) -> i64 {
    keys.iter().find_map(|k| v.get(*k).and_then(Value::as_f64)).map(|f| f as i64).unwrap_or(0)
}

/// `us-east-1` out of a tenant URL like `…dp.appintegrations.us-east-1.prod…`.
pub fn region_of(tenant_url: &str) -> String {
    let re = ["us", "eu", "ap", "sa", "ca", "me", "af", "il"];
    for part in tenant_url.split(['.', '/']) {
        let mut it = part.splitn(3, '-');
        if let (Some(a), Some(b), Some(c)) = (it.next(), it.next(), it.next())
            && re.contains(&a)
            && !b.is_empty()
            && b.chars().all(|ch| ch.is_ascii_lowercase())
            && !c.is_empty()
            && c.chars().all(|ch| ch.is_ascii_digit())
        {
            return part.to_string();
        }
    }
    "us-east-1".into()
}

fn oidc(region: &str, path: &str) -> String {
    format!("https://oidc.{region}.amazonaws.com/{path}")
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Client {
    pub client_id: String,
    pub client_secret: String,
    pub client_secret_expires_at: i64,
}

pub async fn register_client(http: &reqwest::Client, region: &str, issuer_url: &str, redirect_uri: &str) -> Result<Client, ApiError> {
    let body = json!({
        "clientName": CLIENT_NAME,
        "clientType": "public",
        "scopes": [SCOPE],
        "grantTypes": ["authorization_code", "refresh_token"],
        "redirectUris": [redirect_uri],
        "issuerUrl": issuer_url,
    });
    let r = post_json(http, &oidc(region, "client/register"), &body, None).await?;
    let c = Client {
        client_id: s(&r, &["clientId", "client_id"]),
        client_secret: s(&r, &["clientSecret", "client_secret"]),
        client_secret_expires_at: n(&r, &["clientSecretExpiresAt", "client_secret_expires_at"]),
    };
    if c.client_id.is_empty() {
        return Err(ApiError::Http("client registration returned no clientId".into()));
    }
    Ok(c)
}

pub fn authorize_url(region: &str, client_id: &str, redirect_uri: &str, state: &str, challenge: &str) -> String {
    let mut u = url::Url::parse(&oidc(region, "authorize")).expect("static url");
    u.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", SCOPE)
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256");
    u.into()
}

#[derive(Debug, Clone, Default)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    pub id_token: String,
    pub expires_in: i64,
}

fn tokens_of(r: &Value) -> Tokens {
    Tokens {
        access_token: s(r, &["accessToken", "access_token"]),
        refresh_token: s(r, &["refreshToken", "refresh_token"]),
        id_token: s(r, &["idToken", "id_token"]),
        expires_in: n(r, &["expiresIn", "expires_in"]),
    }
}

pub async fn exchange_code(http: &reqwest::Client, region: &str, client: &Client, code: &str, redirect_uri: &str, verifier: &str) -> Result<Tokens, ApiError> {
    let body = json!({
        "clientId": client.client_id,
        "clientSecret": client.client_secret,
        "grantType": "authorization_code",
        "code": code,
        "redirectUri": redirect_uri,
        "codeVerifier": verifier,
    });
    let t = tokens_of(&post_json(http, &oidc(region, "token"), &body, None).await?);
    if t.access_token.is_empty() {
        return Err(ApiError::Http("token response has no accessToken".into()));
    }
    Ok(t)
}

pub async fn refresh(http: &reqwest::Client, region: &str, client_id: &str, client_secret: &str, refresh_token: &str) -> Result<Tokens, ApiError> {
    let body = json!({
        "clientId": client_id,
        "clientSecret": client_secret,
        "grantType": "refresh_token",
        "refreshToken": refresh_token,
    });
    let mut t = tokens_of(&post_json(http, &oidc(region, "token"), &body, None).await?);
    if t.access_token.is_empty() {
        return Err(ApiError::Auth("refresh returned no accessToken".into()));
    }
    if t.refresh_token.is_empty() {
        t.refresh_token = refresh_token.to_string();
    }
    Ok(t)
}

/// Refresh the tokens inside a desktop-format credential blob.
///
/// Every field the desktop stored is kept; only the token fields the server
/// returned are replaced, so a blob written back stays readable by the app.
pub async fn refresh_blob(http: &reqwest::Client, blob: &str) -> Result<String, ApiError> {
    let mut v: Value = serde_json::from_str(blob).map_err(|e| ApiError::Http(format!("凭据不是 JSON：{e}")))?;
    let client_id = s(&v, &["client_id"]);
    let client_secret = s(&v, &["client_secret"]);
    let refresh_token = s(&v, &["refresh_token"]);
    let tenant = s(&v, &["tenant_url", "issuer_url"]);
    if client_id.is_empty() || refresh_token.is_empty() {
        return Err(ApiError::Auth("凭据里没有 client_id 或 refresh_token".into()));
    }
    let t = refresh(http, &region_of(&tenant), &client_id, &client_secret, &refresh_token).await?;
    let lifetime = if t.expires_in > 0 { t.expires_in } else { 3600 };
    let now = chrono::Utc::now().timestamp() as f64;
    let obj = v.as_object_mut().ok_or_else(|| ApiError::Http("stored credentials are not a JSON object".into()))?;
    obj.insert("access_token".into(), Value::String(t.access_token));
    obj.insert("access_token_expiry".into(), Value::from(now + lifetime as f64));
    if !t.refresh_token.is_empty() && t.refresh_token != refresh_token {
        obj.insert("refresh_token".into(), Value::String(t.refresh_token));
    }
    Ok(v.to_string())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct UserInfo {
    pub email: String,
    pub display_name: String,
    pub first_name: String,
    pub last_name: String,
    pub role: String,
    pub aws_account_id: String,
    pub account_name: String,
    pub external_id: String,
    pub account_ownership: String,
    pub user_arn: String,
}

pub fn parse_user_info(raw: &Value) -> UserInfo {
    let v = raw.get("userInfo").filter(|x| x.is_object()).unwrap_or(raw);
    UserInfo {
        email: s(v, &["email", "emailAddress"]),
        display_name: s(v, &["displayName", "display_name"]),
        first_name: s(v, &["firstName", "first_name"]),
        last_name: s(v, &["lastName", "last_name"]),
        role: s(v, &["role"]),
        aws_account_id: s(v, &["awsAccountId", "aws_account_id", "accountId"]),
        account_name: s(v, &["accountName", "account_name"]),
        external_id: s(v, &["externalId", "external_id"]),
        account_ownership: s(v, &["accountOwnership", "account_ownership"]),
        user_arn: s(v, &["userArn", "user_arn"]),
    }
}

fn data_plane(tenant_url: &str) -> String {
    let base = tenant_url.trim_end_matches('/');
    if base.contains("dp.appintegrations") { base.to_string() } else { SOCIAL_DATA_PLANE.to_string() }
}

pub async fn user_info(http: &reqwest::Client, tenant_url: &str, access_token: &str) -> Result<UserInfo, ApiError> {
    let url = format!("{}/integration/quick-work/user-info", data_plane(tenant_url));
    Ok(parse_user_info(&post_json(http, &url, &json!({}), Some(access_token)).await?))
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct UsageWindow {
    pub name: String,
    pub used_hours: f64,
    pub limit_hours: f64,
    pub left_hours: f64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Usage {
    pub plan: String,
    /// unix ms of a free trial's end, 0 when none
    pub trial_ends_at: i64,
    pub windows: Vec<UsageWindow>,
}

/// QUICK_LITE_FREE_TRIAL → "Quick Lite (free trial)".
pub fn plan_name(t: &str) -> String {
    let t = t.trim().to_uppercase();
    if t.is_empty() {
        return String::new();
    }
    let trial = t.ends_with("_FREE_TRIAL");
    let words: Vec<String> = t
        .trim_end_matches("_FREE_TRIAL")
        .split('_')
        .filter(|w| !w.is_empty())
        .map(|w| w[..1].to_string() + &w[1..].to_lowercase())
        .collect();
    let name = words.join(" ");
    if trial { format!("{name} (free trial)") } else { name }
}

fn unit_name(t: &str) -> String {
    match t {
        "AUTOMATION_AGENT_HOUR_UNIT" => "Agent hours".into(),
        "RESEARCH_AGENT_HOUR_UNIT" => "Research hours".into(),
        _ => plan_name(t.trim_end_matches("_UNIT")),
    }
}

fn hours(mins: f64) -> f64 {
    (mins / 60.0 * 100.0).round() / 100.0
}

/// Map a getAvailableEntitlementUnits response. Units drawing on one pool
/// (identical minutes) collapse into one window.
pub fn usage_from(resp: &Value) -> Usage {
    let mut windows: Vec<UsageWindow> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for e in resp.get("entitlements").and_then(Value::as_array).into_iter().flatten() {
        let total = e.get("numMinsProvisioned").and_then(Value::as_f64).unwrap_or(0.0);
        let left = e.get("numMinsAvailable").and_then(Value::as_f64).unwrap_or(0.0);
        if !seen.insert(format!("{total}|{left}")) {
            continue;
        }
        let used = (total - left).max(0.0);
        windows.push(UsageWindow {
            name: unit_name(e.get("unitType").and_then(Value::as_str).unwrap_or("")),
            used_hours: hours(used),
            limit_hours: hours(total),
            left_hours: hours(left.max(0.0)),
        });
    }
    let sub = resp.get("subscriptionMetadata").cloned().unwrap_or(Value::Null);
    let trial = sub.get("freeTrialMetadata").cloned().unwrap_or(Value::Null);
    let active = trial.get("status").and_then(Value::as_str).is_some_and(|s| s.eq_ignore_ascii_case("ACTIVE"));
    Usage {
        plan: plan_name(sub.get("subscriptionType").and_then(Value::as_str).unwrap_or("")),
        trial_ends_at: if active { n(&trial, &["expiryDate"]) } else { 0 },
        windows,
    }
}

pub async fn usage(http: &reqwest::Client, tenant_url: &str, access_token: &str, aws_account_id: &str) -> Result<Usage, ApiError> {
    let ty = "getAvailableEntitlementUnits";
    let url = format!("{}/integration/quick-work/qbs-event", tenant_url.trim_end_matches('/'));
    let body = json!({ "eventType": ty, "eventBody": { ty: { "awsAccountId": aws_account_id } } });
    let r = post_json(http, &url, &body, Some(access_token)).await?;
    let resp = r
        .get("eventResponse")
        .and_then(|e| e.get(format!("{ty}Response")))
        .ok_or_else(|| ApiError::Http(format!("usage query failed: {}", truncate(&s(&r, &["errorMessage"]), 200))))?;
    Ok(usage_from(resp))
}

/// Usage for a stored credential blob, refreshing the access token first when
/// it is missing or about to expire. Returns the (possibly refreshed) blob so
/// the caller can persist a rotated refresh token.
pub async fn usage_for_blob(http: &reqwest::Client, blob: &str) -> Result<(String, Usage), ApiError> {
    let mut v: Value = serde_json::from_str(blob).map_err(|e| ApiError::Http(format!("凭据不是 JSON：{e}")))?;
    let now = chrono::Utc::now().timestamp() as f64;
    let expires = v.get("access_token_expiry").and_then(Value::as_f64).unwrap_or(0.0);
    let missing = v.get("access_token").and_then(Value::as_str).unwrap_or("").is_empty();
    let mut current = blob.to_string();
    if missing || expires <= now + 60.0 {
        current = refresh_blob(http, blob).await?;
        v = serde_json::from_str(&current).map_err(|e| ApiError::Http(format!("凭据不是 JSON：{e}")))?;
    }
    let token = s(&v, &["access_token"]);
    let tenant = s(&v, &["tenant_url"]);
    let aws = s(&v, &["aws_account_id"]);
    if token.is_empty() || tenant.is_empty() || aws.is_empty() {
        return Err(ApiError::Http("凭据里缺少 access_token、tenant_url 或 aws_account_id".into()));
    }
    let usage = usage(http, &tenant, &token, &aws).await?;
    Ok((current, usage))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_from_tenant() {
        assert_eq!(region_of("https://qbs-cellx.dp.appintegrations.us-east-1.prod.plato.ai.aws.dev"), "us-east-1");
        assert_eq!(region_of("https://x.dp.appintegrations.eu-west-2.prod"), "eu-west-2");
        assert_eq!(region_of(""), "us-east-1");
    }

    #[test]
    fn plan_names() {
        assert_eq!(plan_name("QUICK_LITE_FREE_TRIAL"), "Quick Lite (free trial)");
        assert_eq!(plan_name("QUICK_PLUS"), "Quick Plus");
        assert_eq!(plan_name(""), "");
    }

    #[test]
    fn usage_collapses_shared_pools() {
        let r = json!({
            "entitlements": [
                {"unitType": "AUTOMATION_AGENT_HOUR_UNIT", "numMinsAvailable": 529, "numMinsProvisioned": 600},
                {"unitType": "RESEARCH_AGENT_HOUR_UNIT", "numMinsAvailable": 529, "numMinsProvisioned": 600}
            ],
            "subscriptionMetadata": {"subscriptionType": "QUICK_LITE_FREE_TRIAL", "freeTrialMetadata": {"status": "ACTIVE", "expiryDate": 1797000000000i64}}
        });
        let u = usage_from(&r);
        assert_eq!(u.windows.len(), 1);
        assert_eq!(u.windows[0].name, "Agent hours");
        assert_eq!(u.windows[0].limit_hours, 10.0);
        assert_eq!(u.windows[0].used_hours, 1.18);
        assert_eq!(u.plan, "Quick Lite (free trial)");
        assert_eq!(u.trial_ends_at, 1797000000000);
    }

    #[test]
    fn user_info_accepts_wrapped_and_camel_case() {
        let u = parse_user_info(&json!({"userInfo": {"email": "a@x", "externalId": "e-1", "awsAccountId": 123}}));
        assert_eq!(u.email, "a@x");
        assert_eq!(u.external_id, "e-1");
        assert_eq!(u.aws_account_id, "123");
    }

    #[test]
    fn authorize_url_has_pkce() {
        let u = authorize_url("us-east-1", "cid", "http://127.0.0.1:9/", "st", "ch");
        assert!(u.starts_with("https://oidc.us-east-1.amazonaws.com/authorize?"));
        assert!(u.contains("code_challenge=ch") && u.contains("code_challenge_method=S256") && u.contains("state=st"));
    }
}
