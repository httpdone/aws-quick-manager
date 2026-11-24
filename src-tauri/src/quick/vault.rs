//! The tool's own account vault.
//!
//! The desktop's `profiles.json` and credential store stay the desktop's;
//! the vault keeps its own copy of every account. A small index at
//! `<app dir>/accounts.json` holds the fields the UI needs without
//! touching secrets, and each account's credential blob is stored
//! verbatim in the OS credential store under `aws-quick-manager/<id>`
//! (same chunked layout the desktop uses, so the Windows size cap is
//! handled by credstore).

use super::credstore::{self, CredError, RawStore};
use super::device;
use super::fsutil::{self, FileLock};
use super::profiles;
use super::switcher;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const INDEX_FILE: &str = "accounts.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct IdentityChange {
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub config: String,
    #[serde(default)]
    pub space: String,
    #[serde(default)]
    pub replaced_at: String,
}

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("账号库读写失败：{0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Cred(#[from] CredError),
}

fn one() -> u32 {
    1
}

/// The index file's contents.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Index {
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default)]
    pub accounts: Vec<VaultEntry>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One vault account. Unknown fields are kept as-is so a newer version's
/// entries survive a rewrite.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct VaultEntry {
    /// the desktop's profile id (`social-<sha12>`)
    pub id: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub aws_account_id: String,
    #[serde(default)]
    pub tenant_url: String,
    /// RFC3339
    #[serde(default)]
    pub added_at: String,
    /// RFC3339; last time a desktop sync saw this account
    #[serde(default)]
    pub last_synced_at: String,
    /// e.g. "auth_invalid"; unused for now
    #[serde(default)]
    pub flag: Option<String>,
    /// install-level ids this account projects into the base files
    #[serde(default)]
    pub device_configuration_id: String,
    #[serde(default)]
    pub device_space_id: String,
    /// previous full identities, newest last
    #[serde(default)]
    pub identity_history: Vec<IdentityChange>,
    /// the profile's `device_id`, the id the backend reports as its outpost
    /// id; empty until the account is seen on a desktop profile
    #[serde(default)]
    pub device_id: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// What one `import_from_desktop` run did.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct ImportReport {
    pub imported: Vec<String>,
    pub updated: Vec<String>,
    pub unchanged: Vec<String>,
    /// (account id, reason it was skipped)
    pub skipped: Vec<(String, String)>,
}

/// The vault folder: this tool's own app dir.
pub fn dir() -> PathBuf {
    switcher::app_dir()
}

/// Credential-store target for a vault account's session blob.
pub fn secret_target(id: &str) -> String {
    format!("aws-quick-manager/{id}")
}

fn index_path(dir: &Path) -> PathBuf {
    dir.join(INDEX_FILE)
}

/// Read the index; a missing file is an empty vault.
fn read_index(dir: &Path) -> io::Result<Index> {
    match fs::read_to_string(index_path(dir)) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Index { version: 1, ..Default::default() }),
        Err(e) => Err(e),
    }
}

/// Write the index atomically (unique temp file + rename). Callers that
/// read, change and write it back hold [`lock`] for the whole round trip.
fn write_index(dir: &Path, index: &Index) -> io::Result<()> {
    let text = serde_json::to_string_pretty(index).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    fsutil::atomic_write(&index_path(dir), (text + "\n").as_bytes())
}

/// Held by every read-change-write of the index, so quota reads running in
/// parallel (and `aqm`) never overwrite each other's changes. None of the
/// functions that take it call another one that does.
fn lock(dir: &Path) -> io::Result<FileLock> {
    FileLock::acquire(&dir.join(format!("{INDEX_FILE}.lock")))
}

/// The refresh token in a credential blob, "" when there is none.
fn refresh_token_of(blob: &str) -> String {
    serde_json::from_str::<Value>(blob)
        .ok()
        .and_then(|v| v.get("refresh_token").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default()
}

/// Every account in the vault index.
pub fn list(dir: &Path) -> io::Result<Vec<VaultEntry>> {
    Ok(read_index(dir)?.accounts)
}

/// The account's stored credential blob, verbatim. `None` when the vault
/// has no readable secret for `id`.
pub fn get_blob(store: &dyn RawStore, id: &str) -> Result<Option<String>, CredError> {
    credstore::read_session_text(store, &secret_target(id))
}

fn field(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// Insert or refresh `id`'s index entry from `blob`, keeping `added_at`
/// and unknown fields of an existing entry.
fn upsert_entry(index: &mut Index, id: &str, blob: &str, now: &str, clear_flag: bool) {
    let v: Value = serde_json::from_str(blob).unwrap_or(Value::Null);
    let (email, display_name, role, aws_account_id, tenant_url) =
        (field(&v, "email"), field(&v, "display_name"), field(&v, "role"), field(&v, "aws_account_id"), field(&v, "tenant_url"));
    match index.accounts.iter().position(|e| e.id == id) {
        Some(i) => {
            let e = &mut index.accounts[i];
            e.email = email;
            e.display_name = display_name;
            e.role = role;
            e.aws_account_id = aws_account_id;
            e.tenant_url = tenant_url;
            e.last_synced_at = now.into();
            if clear_flag {
                e.flag = None;
            }
        }
        None => index.accounts.push(VaultEntry {
            id: id.into(),
            email,
            display_name,
            role,
            aws_account_id,
            tenant_url,
            added_at: now.into(),
            last_synced_at: now.into(),
            flag: None,
            device_id: String::new(),
            device_configuration_id: String::new(),
            device_space_id: String::new(),
            identity_history: Vec::new(),
            extra: Map::new(),
        }),
    }
}

/// Write `blob` (the desktop-format credential JSON) as the account's
/// vault secret, verbatim, and upsert its index entry.
pub fn upsert(dir: &Path, store: &dyn RawStore, id: &str, blob: &str) -> Result<(), VaultError> {
    let _lock = lock(dir)?;
    credstore::write_session_text(store, &secret_target(id), blob)?;
    let mut index = read_index(dir)?;
    upsert_entry(&mut index, id, blob, &chrono::Utc::now().to_rfc3339(), true);
    write_index(dir, &index)?;
    Ok(())
}

/// One account's index entry, if the vault has it.
pub fn entry(dir: &Path, id: &str) -> io::Result<Option<VaultEntry>> {
    Ok(read_index(dir)?.accounts.into_iter().find(|e| e.id == id))
}

/// Copy one desktop account into the vault. Used by the "local credentials"
/// import in the add-account dialog: the user picked this account explicitly,
/// so the desktop copy replaces whatever the vault had.
pub fn import_one(base: &Path, store: &dyn RawStore, vault_dir: &Path, id: &str) -> Result<bool, VaultError> {
    let _lock = lock(vault_dir)?;
    let manifest = profiles::read(base)?;
    let Some(profile) = manifest.entries.iter().find(|e| e.id == id) else {
        return Ok(false);
    };
    let blob = match credstore::read_session_text(store, &profiles::credential_target(profile)) {
        Ok(Some(blob)) => blob,
        _ => return Ok(false),
    };
    credstore::write_session_text(store, &secret_target(id), &blob)?;
    let mut index = read_index(vault_dir)?;
    upsert_entry(&mut index, id, &blob, &chrono::Utc::now().to_rfc3339(), true);
    if manifest.last_active.as_deref() == Some(id) {
        let (config, space) = base_identity(base);
        if !config.is_empty()
            && let Some(e) = index.accounts.iter_mut().find(|e| e.id == id)
            && e.device_configuration_id.is_empty()
        {
            e.device_configuration_id = config;
            e.device_space_id = space;
        }
    }
    if let Some(d) = device::read(&device::profile_dir(base, profile))
        && let Some(i) = index.accounts.iter().position(|e| e.id == id)
    {
        index.accounts[i].device_id = d;
    }
    write_index(vault_dir, &index)?;
    Ok(true)
}

/// Record or clear an account's status flag (`auth_invalid`, …).
pub fn set_flag(dir: &Path, id: &str, flag: Option<&str>) -> io::Result<()> {
    let _lock = lock(dir)?;
    let mut index = read_index(dir)?;
    if let Some(e) = index.accounts.iter_mut().find(|e| e.id == id) {
        e.flag = flag.map(str::to_string);
        write_index(dir, &index)?;
    }
    Ok(())
}

/// Set the install-level ids without touching history; used when adopting the
/// base pair or importing an account.
pub fn set_identity(dir: &Path, id: &str, device_id: &str, config: &str, space: &str) -> io::Result<()> {
    let _lock = lock(dir)?;
    let mut index = read_index(dir)?;
    if let Some(e) = index.accounts.iter_mut().find(|e| e.id == id)
        && (e.device_id != device_id || e.device_configuration_id != config || e.device_space_id != space)
    {
        e.device_id = device_id.to_string();
        e.device_configuration_id = config.to_string();
        e.device_space_id = space.to_string();
        write_index(dir, &index)?;
    }
    Ok(())
}

/// Record a user-initiated identity change: archive the current bundle and
/// set the new one. Sync-only updates use `set_device_id` / `set_identity`.
pub fn record_identity_change(
    dir: &Path,
    id: &str,
    device_id: &str,
    config: &str,
    space: &str,
    now: &str,
) -> io::Result<()> {
    let _lock = lock(dir)?;
    let mut index = read_index(dir)?;
    if let Some(e) = index.accounts.iter_mut().find(|e| e.id == id) {
        let changed = e.device_id != device_id || e.device_configuration_id != config || e.device_space_id != space;
        if !changed {
            return Ok(());
        }
        if !e.device_id.is_empty() || !e.device_configuration_id.is_empty() || !e.device_space_id.is_empty() {
            let old = IdentityChange {
                device_id: e.device_id.clone(),
                config: e.device_configuration_id.clone(),
                space: e.device_space_id.clone(),
                replaced_at: now.to_string(),
            };
            e.identity_history.retain(|h| {
                h.device_id != old.device_id || h.config != old.config || h.space != old.space
            });
            e.identity_history.push(old);
            if e.identity_history.len() > 20 {
                let extra = e.identity_history.len() - 20;
                e.identity_history.drain(..extra);
            }
        }
        e.device_id = device_id.to_string();
        e.device_configuration_id = config.to_string();
        e.device_space_id = space.to_string();
        write_index(dir, &index)?;
    }
    Ok(())
}

/// Keep the vault's copy of the profile's device id in step with the file.
pub fn set_device_id(dir: &Path, id: &str, device_id: &str) -> io::Result<()> {
    let _lock = lock(dir)?;
    let mut index = read_index(dir)?;
    if let Some(e) = index.accounts.iter_mut().find(|e| e.id == id)
        && e.device_id != device_id
    {
        e.device_id = device_id.to_string();
        write_index(dir, &index)?;
    }
    Ok(())
}

/// Drop the account from the vault: its index entry and every
/// credential-store entry it created (main, chunks, twin).
pub fn remove(dir: &Path, store: &dyn RawStore, id: &str) -> Result<(), VaultError> {
    let _lock = lock(dir)?;
    let mut index = read_index(dir)?;
    let before = index.accounts.len();
    index.accounts.retain(|e| e.id != id);
    if index.accounts.len() != before {
        write_index(dir, &index)?;
    }
    credstore::delete_session(store, &secret_target(id))?;
    Ok(())
}

fn base_identity(base: &Path) -> (String, String) {
    let read = |name: &str| {
        fs::read_to_string(base.join(name))
            .ok()
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    };
    (read("device_configuration_id"), read("device_space_id"))
}

fn blob_expiry(blob: &str) -> f64 {
    serde_json::from_str::<Value>(blob)
        .ok()
        .and_then(|v| v.get("access_token_expiry").and_then(Value::as_f64))
        .unwrap_or(0.0)
}

/// Copy every readable desktop session into the vault. The desktop's
/// `last_active` account always takes the desktop copy; for the others the
/// desktop copy wins only when its `access_token_expiry` is newer (the
/// vault may hold a fresher refreshed copy). Missing or corrupt desktop
/// credentials are skipped and never touch the vault copy. Nothing is
/// written to the desktop's files or credentials.
pub fn import_from_desktop(base: &Path, store: &dyn RawStore, vault_dir: &Path) -> Result<ImportReport, VaultError> {
    let _lock = lock(vault_dir)?;
    let manifest = profiles::read(base)?;
    let mut index = read_index(vault_dir)?;
    let mut report = ImportReport::default();
    let now = chrono::Utc::now().to_rfc3339();

    for e in &manifest.entries {
        let id = e.id.as_str();
        let blob = match credstore::read_session_text(store, &profiles::credential_target(e)) {
            Ok(Some(b)) => b,
            Ok(None) => {
                report.skipped.push((id.into(), "桌面端没有可读取的登录凭据".into()));
                continue;
            }
            Err(err) => {
                report.skipped.push((id.into(), format!("桌面端凭据读取失败：{err}")));
                continue;
            }
        };

        let existing = index.accounts.iter().position(|x| x.id == id);
        let vault_blob = match existing {
            Some(_) => get_blob(store, id).unwrap_or_default(),
            None => None,
        };

        if let Some(vault_blob) = &vault_blob
            && *vault_blob == blob
        {
            if let Some(i) = existing {
                index.accounts[i].last_synced_at = now.clone();
            }
            report.unchanged.push(id.into());
            continue;
        }

        let desktop_wins = manifest.last_active.as_deref() == Some(id)
            || match existing {
                None => true,
                Some(_) => blob_expiry(&blob) > blob_expiry(vault_blob.as_deref().unwrap_or_default()),
            };

        if desktop_wins {
            credstore::write_session_text(store, &secret_target(id), &blob)?;
            // A later expiry alone doesn't mean the login works again: keep a
            // rejected-login flag while Quick still holds the same refresh
            // token, and clear it once Quick has a new login.
            let token = refresh_token_of(&blob);
            let same_login = !token.is_empty() && vault_blob.as_deref().is_some_and(|vb| refresh_token_of(vb) == token);
            upsert_entry(&mut index, id, &blob, &now, !same_login);
            if existing.is_some() {
                report.updated.push(id.into());
            } else {
                report.imported.push(id.into());
            }
        } else if let Some(i) = existing {
            index.accounts[i].last_synced_at = now.clone();
            report.unchanged.push(id.into());
        }
    }

    // the profile's `device_id` is the id the backend reports as its outpost
    // id; keep the vault copy in step with the file the desktop uses
    for e in &manifest.entries {
        if let Some(d) = device::read(&device::profile_dir(base, e))
            && let Some(i) = index.accounts.iter().position(|x| x.id == e.id)
        {
            index.accounts[i].device_id = d;
        }
    }

    // the account the desktop is currently on owns the base pair; adopt it so
    // a later switch doesn't lose the space that account has been using
    let (config, space) = base_identity(base);
    if !config.is_empty()
        && let Some(active) = manifest.last_active.as_deref()
        && let Some(i) = index.accounts.iter().position(|e| e.id == active)
        && index.accounts[i].device_configuration_id.is_empty()
    {
        index.accounts[i].device_configuration_id = config;
        index.accounts[i].device_space_id = space;
    }

    write_index(vault_dir, &index)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quick::credstore::tests::MemStore;
    use crate::quick::profiles::{Manifest, ProfileEntry};
    use sha2::{Digest, Sha256};

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aqm-vault-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    /// A desktop-format credential blob. Deterministic so tests can
    /// compare verbatim.
    fn blob(tag: &str, expiry: f64) -> String {
        serde_json::json!({
            "refresh_token": format!("rt-{tag}"),
            "access_token": format!("at-{tag}"),
            "email": format!("{tag}@example.com"),
            "display_name": format!("Name {tag}"),
            "role": "EXPLORER",
            "aws_account_id": "123456789012",
            "tenant_url": "https://tenant.example.com",
            "access_token_expiry": expiry,
            "refresh_token_expiry": 1800000000.0,
            "some_future_field": {"nested": true}
        })
        .to_string()
    }

    fn desktop_cred(s: &MemStore, entry: &ProfileEntry, text: &str) {
        s.set(&profiles::credential_target(entry), text).unwrap();
    }

    fn entry(id: &str) -> ProfileEntry {
        ProfileEntry { id: id.into(), provider: "social".into(), ..Default::default() }
    }

    fn manifest(entries: Vec<ProfileEntry>, last_active: Option<&str>) -> Manifest {
        Manifest { version: 1, entries, last_active: last_active.map(str::to_string), ..Default::default() }
    }

    fn vault_blob_expiry(s: &MemStore, id: &str) -> f64 {
        blob_expiry(&get_blob(s, id).unwrap().unwrap())
    }

    #[test]
    fn index_roundtrip_keeps_unknown_fields() {
        let dir = temp("idx");
        let mut index = Index { version: 1, ..Default::default() };
        let mut e = VaultEntry { id: "social-a".into(), email: "a@x".into(), flag: Some("auth_invalid".into()), ..Default::default() };
        e.extra.insert("future".into(), Value::from(7));
        index.accounts.push(e);
        index.extra.insert("later".into(), Value::Bool(true));
        write_index(&dir, &index).unwrap();
        let back = read_index(&dir).unwrap();
        assert_eq!(back, index);
        assert_eq!(back.accounts[0].extra["future"], 7);
        assert_eq!(back.extra["later"], true);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn upsert_derives_fields_and_preserves_added_at_and_extra() {
        let dir = temp("upsert");
        let s = MemStore::default();
        upsert(&dir, &s, "social-a", &blob("a", 100.0)).unwrap();
        let e = list(&dir).unwrap().remove(0);
        assert_eq!(e.email, "a@example.com");
        assert_eq!(e.display_name, "Name a");
        assert_eq!(e.role, "EXPLORER");
        assert_eq!(e.aws_account_id, "123456789012");
        assert_eq!(e.tenant_url, "https://tenant.example.com");
        assert!(!e.added_at.is_empty() && e.added_at == e.last_synced_at);
        let added_at = e.added_at.clone();

        // an unknown field survives a later upsert
        let mut index = read_index(&dir).unwrap();
        index.accounts[0].extra.insert("keep".into(), Value::from("me"));
        write_index(&dir, &index).unwrap();

        upsert(&dir, &s, "social-a", &blob("b", 200.0)).unwrap();
        let e = list(&dir).unwrap().remove(0);
        assert_eq!(e.email, "b@example.com");
        assert_eq!(e.added_at, added_at);
        assert_eq!(e.extra["keep"], "me");
        assert_eq!(get_blob(&s, "social-a").unwrap().unwrap(), blob("b", 200.0));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_new_account_is_imported() {
        let base = temp("imp-base");
        let vdir = temp("imp-vault");
        let s = MemStore::default();
        profiles::write(&base, &manifest(vec![entry("social-a")], Some("social-a"))).unwrap();
        desktop_cred(&s, &entry("social-a"), &blob("a", 100.0));

        let r = import_from_desktop(&base, &s, &vdir).unwrap();
        assert_eq!(r.imported, ["social-a"]);
        assert!(r.updated.is_empty() && r.unchanged.is_empty() && r.skipped.is_empty());
        assert_eq!(list(&vdir).unwrap().len(), 1);
        assert_eq!(get_blob(&s, "social-a").unwrap().unwrap(), blob("a", 100.0));
        let _ = fs::remove_dir_all(&base);
        let _ = fs::remove_dir_all(&vdir);
    }

    #[test]
    fn reimport_of_identical_blob_is_unchanged() {
        let base = temp("re-base");
        let vdir = temp("re-vault");
        let s = MemStore::default();
        profiles::write(&base, &manifest(vec![entry("social-a")], Some("social-a"))).unwrap();
        desktop_cred(&s, &entry("social-a"), &blob("a", 100.0));

        import_from_desktop(&base, &s, &vdir).unwrap();
        let r = import_from_desktop(&base, &s, &vdir).unwrap();
        assert_eq!(r.unchanged, ["social-a"]);
        assert!(r.imported.is_empty() && r.updated.is_empty() && r.skipped.is_empty());
        let _ = fs::remove_dir_all(&base);
        let _ = fs::remove_dir_all(&vdir);
    }

    #[test]
    fn merge_rule_last_active_and_expiry() {
        let base = temp("m-base");
        let vdir = temp("m-vault");
        let s = MemStore::default();
        // vault copies: a=500, b=300, c=300, d=500
        for (id, tag, exp) in [("social-a", "va", 500.0), ("social-b", "vb", 300.0), ("social-c", "vc", 300.0), ("social-d", "vd", 500.0)] {
            upsert(&vdir, &s, id, &blob(tag, exp)).unwrap();
        }
        // desktop copies: a=100 (last_active → wins anyway), b=200 (loses), c=400 (wins), d=400 (loses)
        let entries = vec![entry("social-a"), entry("social-b"), entry("social-c"), entry("social-d")];
        profiles::write(&base, &manifest(entries.clone(), Some("social-a"))).unwrap();
        for ((e, tag), exp) in entries.iter().zip(["da", "db", "dc", "dd"]).zip([100.0, 200.0, 400.0, 400.0]) {
            desktop_cred(&s, e, &blob(tag, exp));
        }

        let r = import_from_desktop(&base, &s, &vdir).unwrap();
        assert_eq!(r.updated, ["social-a", "social-c"]);
        assert_eq!(r.unchanged, ["social-b", "social-d"]);
        assert!(r.imported.is_empty() && r.skipped.is_empty());
        // last_active: desktop copy won even though it was older
        assert_eq!(get_blob(&s, "social-a").unwrap().unwrap(), blob("da", 100.0));
        // fresher desktop copy won
        assert_eq!(vault_blob_expiry(&s, "social-c"), 400.0);
        // older desktop copy lost; vault kept its fresher blob
        assert_eq!(get_blob(&s, "social-b").unwrap().unwrap(), blob("vb", 300.0));
        assert_eq!(vault_blob_expiry(&s, "social-d"), 500.0);
        let _ = fs::remove_dir_all(&base);
        let _ = fs::remove_dir_all(&vdir);
    }

    #[test]
    fn corrupt_or_missing_desktop_credential_is_skipped() {
        let base = temp("c-base");
        let vdir = temp("c-vault");
        let s = MemStore::default();
        upsert(&vdir, &s, "social-c", &blob("vc", 300.0)).unwrap();
        profiles::write(&base, &manifest(vec![entry("social-c"), entry("social-m")], None)).unwrap();
        // chunk manifest promises two chunks, only one exists → Corrupt
        let t = profiles::credential_target(&entry("social-c"));
        s.set(&t, r#"{"__chunks__": 2, "__sha__": ""}"#).unwrap();
        s.set(&format!("{t}_chunk_0"), "{}").unwrap();
        // social-m has no credential at all

        let before = list(&vdir).unwrap();
        let r = import_from_desktop(&base, &s, &vdir).unwrap();
        assert_eq!(r.skipped.len(), 2);
        assert!(r.skipped.iter().any(|(id, _)| id == "social-c"));
        assert!(r.skipped.iter().any(|(id, _)| id == "social-m"));
        assert!(r.imported.is_empty() && r.updated.is_empty() && r.unchanged.is_empty());
        // the vault copy is untouched
        assert_eq!(get_blob(&s, "social-c").unwrap().unwrap(), blob("vc", 300.0));
        assert_eq!(list(&vdir).unwrap(), before);
        let _ = fs::remove_dir_all(&base);
        let _ = fs::remove_dir_all(&vdir);
    }

    #[test]
    fn remove_deletes_index_entry_and_every_secret_entry() {
        let dir = temp("rm");
        let s = MemStore::default();
        upsert(&dir, &s, "social-a", &blob("a", 100.0)).unwrap();
        upsert(&dir, &s, "social-b", &blob("b", 100.0)).unwrap();
        assert!(s.0.borrow().keys().any(|k| k.contains("social-a")));

        remove(&dir, &s, "social-a").unwrap();
        assert_eq!(list(&dir).unwrap().len(), 1);
        assert!(get_blob(&s, "social-a").unwrap().is_none());
        assert!(!s.0.borrow().keys().any(|k| k.contains("social-a")));
        // the other account is untouched
        assert!(get_blob(&s, "social-b").unwrap().is_some());
        let _ = fs::remove_dir_all(&dir);
    }

    /// Imports the real desktop credentials into the real vault and checks
    /// the stored blob is byte-identical. Writes only to this tool's own
    /// app dir and `aws-quick-manager/…` credential entries.
    /// Run with `cargo test -- --ignored live_import_from_desktop --nocapture`.
    #[test]
    #[ignore]
    fn live_import_from_desktop() {
        let base = profiles::base_dir();
        let store = credstore::system();
        let report = import_from_desktop(&base, store.as_ref(), &dir()).unwrap();
        println!(
            "imported={} updated={} unchanged={} skipped={}",
            report.imported.len(),
            report.updated.len(),
            report.unchanged.len(),
            report.skipped.len()
        );
        for id in report.imported.iter().chain(report.updated.iter()).chain(report.unchanged.iter()) {
            println!("  synced {id}");
        }
        for (id, why) in &report.skipped {
            println!("  skipped {id}: {why}");
        }

        let id = "social-d8f9937dc6fd";
        let manifest = profiles::read(&base).unwrap();
        let e = manifest.entries.iter().find(|e| e.id == id).expect("account not in profiles.json");
        let desk = credstore::read_session_text(store.as_ref(), &profiles::credential_target(e))
            .unwrap()
            .expect("no desktop credential");
        let vault_blob = get_blob(store.as_ref(), id).unwrap().expect("no vault blob");
        let desk_sha = hex::encode(Sha256::digest(desk.as_bytes()));
        let vault_sha = hex::encode(Sha256::digest(vault_blob.as_bytes()));
        println!("{id}: blob sha256 match={}", desk_sha == vault_sha);
        assert_eq!(desk_sha, vault_sha);
    }

    #[test]
    fn parallel_index_writes_are_not_lost() {
        let dir = temp("parallel");
        let mut index = Index { version: 1, ..Default::default() };
        for i in 0..16 {
            index.accounts.push(VaultEntry { id: format!("social-{i}"), ..Default::default() });
        }
        write_index(&dir, &index).unwrap();
        let handles: Vec<_> = (0..16)
            .map(|i| {
                let dir = dir.clone();
                std::thread::spawn(move || set_device_id(&dir, &format!("social-{i}"), &format!("dev-{i}")).unwrap())
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let back = read_index(&dir).unwrap();
        assert_eq!(back.accounts.len(), 16);
        for (i, e) in back.accounts.iter().enumerate() {
            assert_eq!(e.device_id, format!("dev-{i}"));
        }
        let leftovers: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sync_keeps_a_rejected_login_flag_until_quick_has_a_new_login() {
        let dir = temp("flag");
        let base = temp("flag-base");
        let s = MemStore::default();
        let e = entry("social-a");
        upsert(&dir, &s, "social-a", &blob("a", 100.0)).unwrap();
        set_flag(&dir, "social-a", Some("auth_invalid")).unwrap();
        profiles::write(&base, &manifest(vec![e.clone()], None)).unwrap();
        let flag = || read_index(&dir).unwrap().accounts[0].flag.clone();

        // same refresh token with a later expiry: still the rejected login
        desktop_cred(&s, &e, &blob("a", 200.0));
        import_from_desktop(&base, &s, &dir).unwrap();
        assert_eq!(flag().as_deref(), Some("auth_invalid"));

        // the user signed in again inside Quick: a new refresh token
        desktop_cred(&s, &e, &blob("a", 300.0).replace("rt-a", "rt-a2"));
        import_from_desktop(&base, &s, &dir).unwrap();
        assert_eq!(flag(), None);

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&base);
    }
}
