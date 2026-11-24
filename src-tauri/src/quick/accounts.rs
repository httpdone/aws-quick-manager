//! What the UI shows about each desktop account. Tokens never leave this
//! module: the view carries only names, ids and expiry times.

use super::credstore::{self, RawStore};
use super::device;
use super::profiles::{self, Manifest};
use super::vault::{self, VaultEntry};
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
    /// the account's credentials are also in the tool's vault
    pub in_vault: bool,
    /// the account is in the desktop's profiles.json
    pub on_desktop: bool,
    /// why the credentials couldn't be read, if they couldn't
    pub error: Option<String>,
    /// status recorded by the vault (`auth_invalid`, …)
    pub flag: Option<String>,
}

fn text(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn secs(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(Value::as_f64).map(|f| f as i64).unwrap_or(0)
}

/// A vault-only account's view: index fields plus whatever the stored
/// blob still says (it may be fresher than the index).
fn vault_view(e: &VaultEntry, store: &dyn RawStore) -> AccountView {
    let (blob, error) = match vault::get_blob(store, &e.id) {
        Ok(b) => (b, None),
        Err(err) => (None, Some(err.to_string())),
    };
    let s = blob.as_deref().and_then(|t| serde_json::from_str::<Value>(t).ok()).unwrap_or(Value::Null);
    let pick = |k: &str, fallback: &str| {
        let v = text(&s, k);
        if v.is_empty() { fallback.to_string() } else { v }
    };
    AccountView {
        id: e.id.clone(),
        // the id is `<provider>-<sha12>`; the provider may itself contain '-'
        provider: e.id.rsplit_once('-').map(|(p, _)| p).unwrap_or("social").to_string(),
        display_name: pick("display_name", &e.display_name),
        email: pick("email", &e.email),
        role: pick("role", &e.role),
        aws_account_id: pick("aws_account_id", &e.aws_account_id),
        active: false,
        has_credentials: s.get("refresh_token").is_some(),
        access_expires_at: secs(&s, "access_token_expiry"),
        refresh_expires_at: secs(&s, "refresh_token_expiry"),
        refresh_failing_since: None,
        last_used_at: None,
        in_vault: true,
        on_desktop: false,
        error,
        flag: e.flag.clone(),
    }
}

pub fn list_from(manifest: &Manifest, vault: &[VaultEntry], store: &dyn RawStore) -> Vec<AccountView> {
    let active = manifest.last_active.as_deref().unwrap_or_default();
    let vaulted: std::collections::HashMap<&str, &VaultEntry> = vault.iter().map(|e| (e.id.as_str(), e)).collect();
    let mut views: Vec<AccountView> = manifest
        .entries
        .iter()
        .map(|e| {
            let (session, error) = match credstore::read_session(store, &profiles::credential_target(e)) {
                Ok(s) => (s, None),
                Err(err) => (None, Some(err.to_string())),
            };
            let s = session.clone().unwrap_or(Value::Null);
            let display = text(&s, "display_name");
            // The tool refreshes the vault copy itself (quota reads, switching);
            // Quick only refreshes the account it is running. Show whichever
            // copy is fresher, or every idle account would look expired.
            let in_vault = vaulted.contains_key(e.id.as_str());
            let v = if in_vault {
                vault::get_blob(store, &e.id)
                    .ok()
                    .flatten()
                    .and_then(|b| serde_json::from_str::<Value>(&b).ok())
                    .unwrap_or(Value::Null)
            } else {
                Value::Null
            };
            AccountView {
                id: e.id.clone(),
                provider: e.provider.clone(),
                display_name: if display.is_empty() { e.display_name.clone() } else { display },
                email: text(&s, "email"),
                role: text(&s, "role"),
                aws_account_id: text(&s, "aws_account_id"),
                active: e.id == active,
                has_credentials: session.as_ref().is_some_and(|s| s.get("refresh_token").is_some()),
                access_expires_at: secs(&s, "access_token_expiry").max(secs(&v, "access_token_expiry")),
                refresh_expires_at: secs(&s, "refresh_token_expiry").max(secs(&v, "refresh_token_expiry")),
                // Quick's own "refresh failing" mark; for a vaulted account the
                // tool's flag (set and cleared by its own refreshes) is the
                // current answer, and Quick's mark would never go away
                refresh_failing_since: if in_vault { None } else { e.refresh_failing_since.clone().filter(|v| !v.is_null()) },
                last_used_at: e.last_used_at.clone(),
                in_vault,
                on_desktop: true,
                error,
                flag: vaulted.get(e.id.as_str()).and_then(|v| v.flag.clone()),
            }
        })
        .collect();
    for e in vault {
        if !manifest.entries.iter().any(|m| m.id == e.id) {
            views.push(vault_view(e, store));
        }
    }
    views
}

pub fn list(base: &Path, vault_dir: &Path, store: &dyn RawStore) -> std::io::Result<Vec<AccountView>> {
    let manifest = profiles::read(base)?;
    Ok(list_from(&manifest, &vault::list(vault_dir)?, store))
}

/// Everything the details dialog shows, assembled from the view, the
/// manifest entry and the vault index. Still no tokens.
#[derive(Debug, Clone, Serialize)]
pub struct AccountDetails {
    pub id: String,
    pub provider: String,
    pub display_name: String,
    pub email: String,
    pub role: String,
    pub aws_account_id: String,
    pub tenant_url: String,
    pub client_id: String,
    pub issuer_url: String,
    pub created_at: Option<String>,
    pub last_used_at: Option<String>,
    pub added_at: Option<String>,
    pub last_synced_at: Option<String>,
    pub device_id: String,
    pub access_expires_at: i64,
    pub refresh_expires_at: i64,
    pub active: bool,
    pub in_vault: bool,
    pub on_desktop: bool,
    pub flag: Option<String>,
    pub error: Option<String>,
}

pub fn details(base: &Path, vault_dir: &Path, store: &dyn RawStore, id: &str) -> std::io::Result<Option<AccountDetails>> {
    let Some(view) = list(base, vault_dir, store)?.into_iter().find(|v| v.id == id) else {
        return Ok(None);
    };
    let manifest = profiles::read(base)?;
    let profile = manifest.entries.iter().find(|e| e.id == id);
    let entry = vault::entry(vault_dir, id)?;
    let meta = |key: &str| {
        profile
            .and_then(|p| p.login_metadata.get(key))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let tenant_url = {
        let t = meta("tenant_url");
        if t.is_empty() { entry.as_ref().map(|e| e.tenant_url.clone()).unwrap_or_default() } else { t }
    };
    let device_id = entry
        .as_ref()
        .map(|e| e.device_id.clone())
        .filter(|d| !d.is_empty())
        .or_else(|| profile.and_then(|p| device::read(&device::profile_dir(base, p))))
        .unwrap_or_default();
    Ok(Some(AccountDetails {
        id: view.id,
        provider: view.provider,
        display_name: view.display_name,
        email: view.email,
        role: view.role,
        aws_account_id: view.aws_account_id,
        tenant_url,
        client_id: meta("client_id"),
        issuer_url: meta("issuer_url"),
        created_at: profile.and_then(|p| p.created_at.clone()),
        last_used_at: view.last_used_at,
        added_at: entry.as_ref().map(|e| e.added_at.clone()),
        last_synced_at: entry.as_ref().map(|e| e.last_synced_at.clone()),
        device_id,
        access_expires_at: view.access_expires_at,
        refresh_expires_at: view.refresh_expires_at,
        active: view.active,
        in_vault: view.in_vault,
        on_desktop: view.on_desktop,
        flag: view.flag,
        error: view.error,
    }))
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
        let vault = vec![VaultEntry { id: "social-a".into(), ..Default::default() }];
        let v = list_from(&m, &vault, &s);
        assert_eq!(v[0].display_name, "Ann");
        assert!(v[0].has_credentials && !v[0].active);
        assert_eq!(v[0].access_expires_at, 1700000000);
        assert!(v[0].in_vault && v[0].on_desktop);
        assert!(!v[1].has_credentials && v[1].active);
        assert!(!v[1].in_vault && v[1].on_desktop);
        assert_eq!(v[1].display_name, "Bob");
        assert!(!serde_json::to_string(&v).unwrap().contains("SECRET"));
    }

    #[test]
    fn view_includes_vault_only_accounts_without_tokens() {
        let s = MemStore::default();
        s.set(
            "aws-quick-manager/social-v",
            r#"{"refresh_token":"SECRET","access_token":"SECRET","email":"v@x","role":"ADMIN","access_token_expiry":1700000000,"refresh_token_expiry":1800000000}"#,
        )
        .unwrap();
        let m = Manifest { version: 1, ..Default::default() };
        let vault = vec![
            VaultEntry {
                id: "social-v".into(),
                email: "v@x".into(),
                display_name: "Vera".into(),
                role: "ADMIN".into(),
                added_at: "2026-01-01T00:00:00Z".into(),
                last_synced_at: "2026-01-02T00:00:00Z".into(),
                ..Default::default()
            },
            VaultEntry { id: "social-gone".into(), email: "g@x".into(), ..Default::default() },
        ];
        let v = list_from(&m, &vault, &s);
        assert_eq!(v.len(), 2);
        assert!(v.iter().all(|a| a.in_vault && !a.on_desktop && !a.active));
        assert_eq!(v[0].provider, "social");
        assert_eq!(v[0].email, "v@x");
        assert_eq!(v[0].display_name, "Vera");
        assert!(v[0].has_credentials);
        assert_eq!(v[0].access_expires_at, 1700000000);
        // index entry without a stored secret: listed, but not usable
        assert!(!v[1].has_credentials && v[1].email == "g@x");
        assert!(!serde_json::to_string(&v).unwrap().contains("SECRET"));
    }

    #[test]
    fn vaulted_account_shows_the_fresher_copy_and_drops_quicks_failure_mark() {
        let s = MemStore::default();
        // Quick's copy is stale: it only refreshes the account it is running
        s.set("quickwork-social-social-a", r#"{"refresh_token":"r","access_token_expiry":100,"refresh_token_expiry":200}"#).unwrap();
        // the tool refreshed its own copy since
        s.set("aws-quick-manager/social-a", r#"{"refresh_token":"r","access_token_expiry":5000,"refresh_token_expiry":9000}"#).unwrap();
        let failing = Some(serde_json::json!("2026-01-01T00:00:00Z"));
        let m = Manifest {
            version: 1,
            entries: vec![
                ProfileEntry { id: "social-a".into(), provider: "social".into(), refresh_failing_since: failing.clone(), ..Default::default() },
                ProfileEntry { id: "social-b".into(), provider: "social".into(), refresh_failing_since: failing, ..Default::default() },
            ],
            ..Default::default()
        };
        let vault = vec![VaultEntry { id: "social-a".into(), ..Default::default() }];
        let v = list_from(&m, &vault, &s);
        assert_eq!(v[0].access_expires_at, 5000);
        assert_eq!(v[0].refresh_expires_at, 9000);
        // the tool's own flag decides for vaulted accounts
        assert!(v[0].refresh_failing_since.is_none());
        // an account only in Quick keeps Quick's mark
        assert!(v[1].refresh_failing_since.is_some());
    }

    /// Reads the real store; run with `cargo test -- --ignored live`.
    #[test]
    #[ignore]
    fn live_list_reads_this_machine() {
        let v = list(&profiles::base_dir(), &vault::dir(), credstore::system().as_ref()).unwrap();
        for a in &v {
            println!("{} active={} creds={} vault={} desktop={} role={} access_exp={} refresh_exp={} err={:?}", a.id, a.active, a.has_credentials, a.in_vault, a.on_desktop, a.role, a.access_expires_at, a.refresh_expires_at, a.error);
        }
        assert!(v.iter().any(|a| a.has_credentials));
    }
}
