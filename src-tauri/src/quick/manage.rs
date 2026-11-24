//! Delete an account and reset its device id.
//!
//! Both can change files the desktop reads, so both refuse to run while the
//! desktop is open: a quitting desktop may rewrite the manifest or its
//! profile on the way out.

use super::credstore::{self, RawStore};
use super::desktop::{self, DesktopError};
use super::device;
use super::profiles::{self, MANIFEST_FILE};
use super::switcher;
use super::vault;
use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ManageError {
    #[error("桌面端还开着，先退出 Quick 再操作")]
    DesktopRunning,
    #[error(transparent)]
    Desktop(#[from] DesktopError),
    #[error(transparent)]
    Vault(#[from] vault::VaultError),
    #[error(transparent)]
    Cred(#[from] credstore::CredError),
    #[error("读写文件失败：{0}")]
    Io(#[from] io::Error),
}

/// Remove an account from the vault, and optionally from the desktop:
/// manifest entry, credential-store session and profile data directory.
pub fn delete(
    base: &Path,
    backups: &Path,
    store: &dyn RawStore,
    vault_dir: &Path,
    id: &str,
    remove_desktop: bool,
    remove_data: bool,
) -> Result<(), ManageError> {
    if !remove_desktop {
        vault::remove(vault_dir, store, id)?;
        return Ok(());
    }

    if desktop::find()?.is_some() {
        return Err(ManageError::DesktopRunning);
    }
    delete_from_desktop(base, backups, store, vault_dir, id, remove_data)
}

/// The desktop part of [`delete`], once Quick is known to be closed.
fn delete_from_desktop(base: &Path, backups: &Path, store: &dyn RawStore, vault_dir: &Path, id: &str, remove_data: bool) -> Result<(), ManageError> {
    let mut manifest = profiles::read(base)?;
    let profile = manifest.entries.iter().find(|e| e.id == id).cloned();

    if let Some(profile) = &profile {
        if base.join(MANIFEST_FILE).is_file() {
            switcher::backup_if_changed(base, backups)?;
        }
        manifest.entries.retain(|e| e.id != id);
        if manifest.last_active.as_deref() == Some(id) {
            manifest.last_active = manifest.entries.first().map(|e| e.id.clone());
        }
        profiles::write(base, &manifest)?;
        credstore::delete_session(store, &profiles::credential_target(profile))?;
        if remove_data {
            let dir = device::profile_dir(base, profile);
            if dir.exists() {
                fs::remove_dir_all(dir)?;
            }
        }
    }

    vault::remove(vault_dir, store, id)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quick::credstore::tests::MemStore;
    use crate::quick::profiles::{Manifest, ProfileEntry};
    use std::path::PathBuf;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aqm-manage-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    fn entry(id: &str) -> ProfileEntry {
        ProfileEntry { id: id.into(), provider: "social".into(), ..Default::default() }
    }

    fn setup(name: &str) -> (PathBuf, MemStore) {
        let root = temp(name);
        let base = root.join("base");
        let store = MemStore::default();
        let m = Manifest { version: 1, entries: vec![entry("social-a"), entry("social-b")], last_active: Some("social-a".into()), ..Default::default() };
        profiles::write(&base, &m).unwrap();
        for e in &m.entries {
            store.set(&profiles::credential_target(e), r#"{"refresh_token":"r"}"#).unwrap();
            device::write_id(&device::profile_dir(&base, e), "dev").unwrap();
            vault::upsert(&root.join("vault"), &store, &e.id, r#"{"refresh_token":"r"}"#).unwrap();
        }
        (root, store)
    }

    #[test]
    fn vault_only_delete_leaves_quick_alone() {
        let (root, store) = setup("vault-only");
        let (base, vault_dir) = (root.join("base"), root.join("vault"));
        delete(&base, &root.join("backups"), &store, &vault_dir, "social-a", false, false).unwrap();
        assert!(vault::list(&vault_dir).unwrap().iter().all(|e| e.id != "social-a"));
        assert_eq!(profiles::read(&base).unwrap().entries.len(), 2);
        assert!(store.get(&profiles::credential_target(&entry("social-a"))).unwrap().is_some());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn deleting_the_active_account_from_quick_moves_last_active_and_backs_up() {
        let (root, store) = setup("desktop");
        let (base, vault_dir, backups) = (root.join("base"), root.join("vault"), root.join("backups"));
        delete_from_desktop(&base, &backups, &store, &vault_dir, "social-a", true).unwrap();

        let m = profiles::read(&base).unwrap();
        assert_eq!(m.entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), vec!["social-b"]);
        assert_eq!(m.last_active.as_deref(), Some("social-b"));
        assert!(store.get(&profiles::credential_target(&entry("social-a"))).unwrap().is_none());
        assert!(!device::profile_dir(&base, &entry("social-a")).exists());
        assert!(device::profile_dir(&base, &entry("social-b")).exists());
        assert!(vault::list(&vault_dir).unwrap().iter().all(|e| e.id != "social-a"));
        assert!(switcher::latest_backup(&backups).unwrap().is_some());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn data_folder_stays_unless_asked() {
        let (root, store) = setup("keep-data");
        let base = root.join("base");
        delete_from_desktop(&base, &root.join("backups"), &store, &root.join("vault"), "social-b", false).unwrap();
        assert!(device::profile_dir(&base, &entry("social-b")).exists());
        assert_eq!(profiles::read(&base).unwrap().last_active.as_deref(), Some("social-a"));
        let _ = fs::remove_dir_all(&root);
    }
}
