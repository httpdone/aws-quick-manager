//! Switch the account the desktop starts with.
//!
//! The desktop reads `last_active` from the manifest at start-up and keeps
//! every account's session in the credential store. A switch is: refresh the
//! target's tokens → quit the desktop and its Python backend → read back what
//! the old account left behind → back up the manifest → write the target's
//! session where the desktop reads it → point `last_active` at it → start the
//! desktop again.

use super::api;
use super::credstore::{self, RawStore};
use super::desktop::{self, DesktopError};
use super::device;
use super::device_ids;
use super::profiles::{self, Manifest, ProfileEntry};
use super::vault;
use serde::Serialize;
use serde_json::{Map, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const KEEP_BACKUPS: usize = 30;

#[derive(Debug, thiserror::Error)]
pub enum SwitchError {
    #[error("没有这个账号：{0}")]
    NoSuchAccount(String),
    #[error("已经是当前账号")]
    AlreadyActive,
    #[error("没有可恢复的备份")]
    NoBackup,
    #[error("桌面端还开着，先退出 Quick 再恢复")]
    DesktopRunning,
    #[error("账号库里没有 {0} 的登录凭据，桌面端也没有可读取的登录信息")]
    NoCredentials(String),
    #[error("刷新登录令牌失败，没有切换：{0}")]
    RefreshFailed(String),
    #[error("写回桌面端凭据失败：{0}")]
    Cred(String),
    #[error("凭据写入后读回不一致，已停止；清单备份在 {0}")]
    VerifyFailed(String),
    #[error("设备标识读写失败：{0}")]
    Device(String),
    #[error("{0}")]
    DeviceConflict(String),
    #[error(transparent)]
    Desktop(#[from] DesktopError),
    #[error("读写文件失败：{0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SwitchReport {
    /// where the previous manifest was saved
    pub backup: String,
    pub was_running: bool,
    pub launched: bool,
    /// something the user has to do by hand
    pub note: Option<String>,
}

/// This tool's own folder (backups, config), outside the desktop's.
pub fn app_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("aws-quick-manager")
}

pub fn backups_dir() -> PathBuf {
    app_dir().join("backups")
}

#[allow(dead_code)] // exercised by the manifest tests; commit() builds entries itself
pub fn set_active(m: &mut Manifest, id: &str) -> Result<(), SwitchError> {
    if !m.entries.iter().any(|e| e.id == id) {
        return Err(SwitchError::NoSuchAccount(id.into()));
    }
    if m.last_active.as_deref() == Some(id) {
        return Err(SwitchError::AlreadyActive);
    }
    m.last_active = Some(id.into());
    Ok(())
}

/// Backup folders, oldest first (names are UTC timestamps).
fn backup_dirs(backups: &Path) -> io::Result<Vec<PathBuf>> {
    let mut dirs: Vec<PathBuf> = match fs::read_dir(backups) {
        Ok(rd) => rd.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.is_dir()).collect(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e),
    };
    dirs.sort();
    Ok(dirs)
}

fn prune(backups: &Path, keep: usize) -> io::Result<()> {
    let dirs = backup_dirs(backups)?;
    let extra = dirs.len().saturating_sub(keep);
    for d in &dirs[..extra] {
        fs::remove_dir_all(d)?;
    }
    Ok(())
}

/// Copy the current manifest into a new timestamped backup folder.
pub fn backup(base: &Path, backups: &Path) -> io::Result<PathBuf> {
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%3fZ").to_string();
    let mut dir = backups.join(&stamp);
    let mut i = 1;
    while dir.exists() {
        dir = backups.join(format!("{stamp}-{i}"));
        i += 1;
    }
    fs::create_dir_all(&dir)?;
    let dst = dir.join(profiles::MANIFEST_FILE);
    fs::copy(base.join(profiles::MANIFEST_FILE), &dst)?;
    prune(backups, KEEP_BACKUPS)?;
    Ok(dst)
}

/// Back up the manifest unless the newest backup is already byte-identical
/// (repeated manual backups, or a switch that did not change anything).
pub fn backup_if_changed(base: &Path, backups: &Path) -> io::Result<PathBuf> {
    let source = fs::read(base.join(profiles::MANIFEST_FILE))?;
    if let Some(latest) = latest_backup(backups)? {
        if fs::read(&latest).map(|previous| previous == source).unwrap_or(false) {
            return Ok(latest);
        }
    }
    backup(base, backups)
}

pub fn latest_backup(backups: &Path) -> io::Result<Option<PathBuf>> {
    Ok(backup_dirs(backups)?.into_iter().rev().map(|d| d.join(profiles::MANIFEST_FILE)).find(|f| f.is_file()))
}

/// Put a backed-up manifest back (temp file + rename).
pub fn restore_file(base: &Path, src: &Path) -> io::Result<()> {
    let tmp = base.join(format!("{}.restore.tmp", profiles::MANIFEST_FILE));
    fs::copy(src, &tmp)?;
    fs::rename(tmp, base.join(profiles::MANIFEST_FILE))
}

/// Refresh the target's tokens before anything is touched.
///
/// The refreshed copy is written to the vault; on failure the switch stops
/// before the desktop is interrupted, and a rejected login is flagged.
pub async fn refresh_target(base: &Path, id: &str, vault_dir: &Path) -> Result<String, SwitchError> {
    if profiles::read(base)?.last_active.as_deref() == Some(id) {
        return Err(SwitchError::AlreadyActive);
    }

    let blob = {
        let store = credstore::system();
        vault::get_blob(store.as_ref(), id).map_err(|e| SwitchError::Cred(e.to_string()))?
    };
    let blob = match blob {
        Some(b) => b,
        None => {
            let store = credstore::system();
            vault::import_from_desktop(base, store.as_ref(), vault_dir).map_err(|e| SwitchError::Cred(e.to_string()))?;
            vault::get_blob(store.as_ref(), id)
                .map_err(|e| SwitchError::Cred(e.to_string()))?
                .ok_or_else(|| SwitchError::NoCredentials(id.into()))?
        }
    };

    let blob = match api::refresh_blob(&api::client(), &blob).await {
        Ok(b) => b,
        Err(e) => {
            // only a rejected login is worth remembering; a network blip isn't
            if let api::ApiError::Auth(message) = &e {
                let flag = if message.contains("403") { "forbidden" } else { "auth_invalid" };
                let _ = vault::set_flag(vault_dir, id, Some(flag));
            }
            return Err(SwitchError::RefreshFailed(e.to_string()));
        }
    };
    {
        let store = credstore::system();
        vault::upsert(vault_dir, store.as_ref(), id, &blob).map_err(|e| SwitchError::Cred(e.to_string()))?;
    }
    Ok(blob)
}

/// Write a refreshed session into the desktop's credential store and read it
/// back. Used when the desktop is closed and the vault has a fresher token
/// (for example after a quota read refreshed it), so the next `last_active`
/// sync doesn't replace the vault copy with an older one.
pub fn write_desktop_blob(base: &Path, store: &dyn RawStore, id: &str, blob: &str) -> Result<(), SwitchError> {
    let manifest = profiles::read(base)?;
    let entry = manifest
        .entries
        .iter()
        .find(|e| e.id == id)
        .ok_or_else(|| SwitchError::NoSuchAccount(id.into()))?;
    let target = profiles::credential_target(entry);
    credstore::write_session_text(store, &target, blob).map_err(|e| SwitchError::Cred(e.to_string()))?;
    let back = credstore::read_session_text(store, &target).map_err(|e| SwitchError::Cred(e.to_string()))?;
    if back.as_deref() != Some(blob) {
        return Err(SwitchError::Cred("写入桌面端凭据后读回不一致".into()));
    }
    Ok(())
}

/// Finish a switch once the target's tokens are fresh. Runs off the async
/// runtime because quitting the desktop can take a while.
pub fn commit(
    base: &Path,
    backups: &Path,
    id: &str,
    launch: bool,
    store: &dyn RawStore,
    vault_dir: &Path,
    blob: &str,
) -> Result<SwitchReport, SwitchError> {
    if let Some(other) = device_ids::conflict_for(base, vault_dir, id)? {
        return Err(SwitchError::DeviceConflict(format!(
            "设备 ID 和账号 {other} 重复，先到设备 ID 页处理"
        )));
    }

    // give the account being left the base pair it has been using, and make
    // sure the target has its own complete config/space before anything quits
    let previous = profiles::read(base)?.last_active.clone();
    device_ids::prepare_switch(base, vault_dir, id, previous.as_deref())
        .map_err(|e| SwitchError::Device(e.to_string()))?;

    let running = desktop::find()?;
    if let Some(p) = &running {
        desktop::quit(p, desktop::QUIT_TIMEOUT)?;
    }

    // the account that just quit may have refreshed its tokens on the way out
    let mut note = match vault::import_from_desktop(base, store, vault_dir) {
        Ok(_) => None,
        Err(e) => Some(format!("没读回桌面端的登录信息：{e}")),
    };

    // the desktop may have rewritten the manifest too
    let mut m = profiles::read(base)?;
    if m.last_active.as_deref() == Some(id) {
        return Err(SwitchError::AlreadyActive);
    }
    let saved = backup_if_changed(base, backups)?;

    let vault_entry = vault::list(vault_dir).ok().and_then(|v| v.into_iter().find(|e| e.id == id));
    let blob_json: Value = serde_json::from_str(blob).unwrap_or(Value::Null);
    if !m.entries.iter().any(|e| e.id == id) {
        // a vault-only account (added through the browser login) gets its
        // manifest entry and profile directory now, at switch time
        let Some(ve) = vault_entry.as_ref() else {
            return Err(SwitchError::NoSuchAccount(id.into()));
        };
        let text = |key: &str| blob_json.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        let issuer = text("issuer_url");
        let tenant = {
            let t = text("tenant_url");
            if t.is_empty() { ve.tenant_url.clone() } else { t }
        };
        let client_id = text("client_id");
        let display = {
            let d = text("display_name");
            if d.is_empty() { ve.display_name.clone() } else { d }
        };
        let mut login_metadata = Map::new();
        if !issuer.is_empty() {
            login_metadata.insert("issuer_url".into(), Value::String(issuer));
        }
        if !tenant.is_empty() {
            login_metadata.insert("tenant_url".into(), Value::String(tenant));
        }
        if !client_id.is_empty() {
            login_metadata.insert("client_id".into(), Value::String(client_id));
        }
        m.entries.push(ProfileEntry {
            id: id.into(),
            provider: id.rsplit_once('-').map(|(p, _)| p).unwrap_or("social").to_string(),
            display_name: display,
            data_path: format!("profiles/{id}"),
            login_metadata,
            created_at: Some(chrono::Utc::now().to_rfc3339()),
            last_used_at: Some(chrono::Utc::now().to_rfc3339()),
            refresh_failing_since: None,
            extra: Map::new(),
        });
    }
    if let Some(entry) = m.entries.iter_mut().find(|e| e.id == id) {
        entry.last_used_at = Some(chrono::Utc::now().to_rfc3339());
        entry.refresh_failing_since = None;
    }
    m.last_active = Some(id.into());

    // write the refreshed session where the desktop reads it, then read it
    // back: a silent truncation here would strand the app on the login page
    let (target, profile) = {
        let entry = m.entries.iter().find(|e| e.id == id).expect("validated above");
        (profiles::credential_target(entry), device::profile_dir(base, entry))
    };

    // the backend reports the profile's `device_id` as its outpost id; keep
    // that id stable and never replace one that is already in use
    let known = vault_entry.map(|e| e.device_id).filter(|d| !d.is_empty());
    let used = device_ids::used_device_ids(base, vault_dir, id)?;
    let device_id = device::ensure_unique(&profile, known.as_deref(), &used).map_err(|e| {
        if e.kind() == io::ErrorKind::InvalidInput {
            SwitchError::DeviceConflict(format!("{}，先到设备 ID 页处理", e))
        } else {
            SwitchError::Device(e.to_string())
        }
    })?;
    if known.as_deref() != Some(device_id.as_str()) {
        let _ = vault::set_device_id(vault_dir, id, &device_id);
    }

    // project the target's install ids into the base files; Quick reads them
    // at startup, and each account keeps its own pair in the vault
    device_ids::write_base_identity(base, vault_dir, id)
        .map_err(|e| SwitchError::Device(e.to_string()))?;

    credstore::write_session_text(store, &target, blob).map_err(|e| SwitchError::Cred(e.to_string()))?;
    let read_back = credstore::read_session_text(store, &target).map_err(|e| SwitchError::Cred(e.to_string()))?;
    if read_back.as_deref() != Some(blob) {
        return Err(SwitchError::VerifyFailed(saved.display().to_string()));
    }

    profiles::write(base, &m)?;

    let mut report = SwitchReport { backup: saved.display().to_string(), was_running: running.is_some(), launched: false, note: None };
    if launch {
        match desktop::installed_exe() {
            Some(exe) => match desktop::launch(&exe) {
                Ok(()) => report.launched = true,
                Err(e) => note = Some(push_note(note, format!("已切换，但{e}，请手动打开 Quick"))),
            },
            None => note = Some(push_note(note, "已切换，但没找到桌面端安装位置，请手动打开 Quick".into())),
        }
    }
    report.note = note;
    Ok(report)
}

fn push_note(note: Option<String>, extra: String) -> String {
    match note {
        Some(n) => format!("{n}；{extra}"),
        None => extra,
    }
}

/// Undo the last switch: put the newest backup back.
pub fn restore_latest(base: &Path, backups: &Path) -> Result<PathBuf, SwitchError> {
    if desktop::find()?.is_some() {
        return Err(SwitchError::DesktopRunning);
    }
    let src = latest_backup(backups)?.ok_or(SwitchError::NoBackup)?;
    restore_file(base, &src)?;
    Ok(src)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quick::profiles::ProfileEntry;

    fn manifest() -> Manifest {
        Manifest {
            version: 1,
            entries: vec![
                ProfileEntry { id: "social-a".into(), ..Default::default() },
                ProfileEntry { id: "social-b".into(), ..Default::default() },
            ],
            last_active: Some("social-a".into()),
            ..Default::default()
        }
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aqm-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn set_active_checks_target() {
        let mut m = manifest();
        assert!(matches!(set_active(&mut m, "social-x"), Err(SwitchError::NoSuchAccount(_))));
        assert!(matches!(set_active(&mut m, "social-a"), Err(SwitchError::AlreadyActive)));
        set_active(&mut m, "social-b").unwrap();
        assert_eq!(m.last_active.as_deref(), Some("social-b"));
    }

    #[test]
    fn backup_then_restore_brings_back_the_old_manifest() {
        let base = temp("base");
        let backups = temp("backups");
        let original = manifest();
        profiles::write(&base, &original).unwrap();

        backup(&base, &backups).unwrap();
        let mut m = original.clone();
        set_active(&mut m, "social-b").unwrap();
        profiles::write(&base, &m).unwrap();
        assert_eq!(profiles::read(&base).unwrap().last_active.as_deref(), Some("social-b"));

        let src = latest_backup(&backups).unwrap().unwrap();
        restore_file(&base, &src).unwrap();
        assert_eq!(profiles::read(&base).unwrap(), original);

        let _ = fs::remove_dir_all(&base);
        let _ = fs::remove_dir_all(&backups);
    }

    #[test]
    fn old_backups_are_pruned() {
        let base = temp("prune-base");
        let backups = temp("prune-backups");
        profiles::write(&base, &manifest()).unwrap();
        for _ in 0..KEEP_BACKUPS + 3 {
            backup(&base, &backups).unwrap();
        }
        assert_eq!(backup_dirs(&backups).unwrap().len(), KEEP_BACKUPS);
        let _ = fs::remove_dir_all(&base);
        let _ = fs::remove_dir_all(&backups);
    }

    #[test]
    fn no_backup_is_none() {
        assert!(latest_backup(&temp("none")).unwrap().is_none());
    }
}
