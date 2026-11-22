//! What the UI shows about each desktop account. Tokens never leave this
//! module: the view carries only names, ids and expiry times.

use super::credstore::{self, RawStore};
use super::profiles::{self, Manifest};
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AccountView {
    pub id: String,
    pub provider: String,
    pub display_name: String,
    pub email: String,
    pub role: String,
    pub aws_account_id: String,
    pub active: bool,
    /// a session is in the credential store
    pub has_credentials: bool,
    /// unix seconds; 0 when unknown
    pub access_expires_at: i64,
    pub refresh_expires_at: i64,
    pub refresh_failing_since: Option<Value>,
    pub last_used_at: Option<String>,
    /// why the credentials couldn't be read, if they couldn't
    pub error: Option<String>,
}

fn text(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn secs(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(Value::as_f64).map(|f| f as i64).unwrap_or(0)
}

pub fn list_from(manifest: &Manifest, store: &dyn RawStore) -> Vec<AccountView> {
    let active = manifest.last_active.as_deref().unwrap_or_default();
    manifest
        .entries
        .iter()
        .map(|e| {
            let (session, error) = match credstore::read_session(store, &profiles::credential_target(e)) {
                Ok(s) => (s, None),
                Err(err) => (None, Some(err.to_string())),
            };
            let s = session.clone().unwrap_or(Value::Null);
            let display = text(&s, "display_name");
            AccountView {
                id: e.id.clone(),
                provider: e.provider.clone(),
                display_name: if display.is_empty() { e.display_name.clone() } else { display },
                email: text(&s, "email"),
                role: text(&s, "role"),
                aws_account_id: text(&s, "aws_account_id"),
                active: e.id == active,
                has_credentials: session.as_ref().is_some_and(|s| s.get("refresh_token").is_some()),
                access_expires_at: secs(&s, "access_token_expiry"),
                refresh_expires_at: secs(&s, "refresh_token_expiry"),
                refresh_failing_since: e.refresh_failing_since.clone().filter(|v| !v.is_null()),
                last_used_at: e.last_used_at.clone(),
                error,
            }
        })
        .collect()
}

pub fn list(base: &Path, store: &dyn RawStore) -> std::io::Result<Vec<AccountView>> {
    Ok(list_from(&profiles::read(base)?, store))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quick::credstore::tests::MemStore;
    use crate::quick::profiles::ProfileEntry;

    #[test]
    fn view_joins_manifest_and_store_without_tokens() {
        let s = MemStore::default();
        s.set(
            "quickwork-social-social-a",
            r#"{"refresh_token":"SECRET","access_token":"SECRET","email":"a@x","display_name":"Ann","role":"EXPLORER","access_token_expiry":1700000000.5,"refresh_token_expiry":1800000000}"#,
        )
        .unwrap();
        let m = Manifest {
            version: 1,
            entries: vec![
                ProfileEntry { id: "social-a".into(), provider: "social".into(), display_name: "old".into(), ..Default::default() },
                ProfileEntry { id: "social-b".into(), provider: "social".into(), display_name: "Bob".into(), ..Default::default() },
            ],
            last_active: Some("social-b".into()),
            ..Default::default()
        };
        let v = list_from(&m, &s);
        assert_eq!(v[0].display_name, "Ann");
        assert!(v[0].has_credentials && !v[0].active);
        assert_eq!(v[0].access_expires_at, 1700000000);
        assert!(!v[1].has_credentials && v[1].active);
        assert_eq!(v[1].display_name, "Bob");
        assert!(!serde_json::to_string(&v).unwrap().contains("SECRET"));
    }

    /// Reads the real store; run with `cargo test -- --ignored live`.
    #[test]
    #[ignore]
    fn live_list_reads_this_machine() {
        let v = list(&profiles::base_dir(), credstore::system().as_ref()).unwrap();
        for a in &v {
            println!("{} active={} creds={} role={} access_exp={} refresh_exp={} err={:?}", a.id, a.active, a.has_credentials, a.role, a.access_expires_at, a.refresh_expires_at, a.error);
        }
        assert!(v.iter().any(|a| a.has_credentials));
    }
}
