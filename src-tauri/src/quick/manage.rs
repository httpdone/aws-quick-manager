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
