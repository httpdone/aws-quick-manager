//! One device identity per account.
//!
//! An identity is three values:
//!   device_id                 profile-scoped; the agent reports it as outpost id
//!   device_configuration_id   install-level; written into the base dir on switch
//!   device_space_id           uuid5 of the config id; keys synced device spaces
//!
//! Only one account runs at a time, so the base files hold the active
//! account's config/space. Switching swaps that pair; the vault keeps each
//! account's own pair and its history. Replacing an identity on an inactive
//! account only changes the vault; on the active account it also writes the
//! profile and base files.

use super::accounts;
use super::credstore::RawStore;
use super::desktop::{self, DesktopError};
use super::device;
use super::profiles;
use super::vault::{self, IdentityChange, VaultEntry};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum DeviceIdError {
    #[error("桌面端还开着，先退出 Quick 再改设备 ID")]
    DesktopRunning,
    #[error("没有这个账号：{0}")]
    NoAccount(String),
    #[error("历史里没有这套设备 ID")]
    NotInHistory,
    #[error("设备 ID {0} 已经被另一个账号使用")]
    Conflict(String),
    #[error("安装 ID {0} 已经被另一个账号使用")]
    ConfigConflict(String),
    #[error("账号 {0} 的 space 和 config 推导值不一致，先修复再切换")]
    ConfigDrift(String),
    #[error("读写文件失败：{0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Desktop(#[from] DesktopError),
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceIdentity {
    pub device_id: String,
    pub config: String,
    pub space: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceAccount {
    pub id: String,
    pub display_name: String,
    pub email: String,
    pub active: bool,
    pub on_desktop: bool,
    pub profile_device_id: Option<String>,
    pub vault_device_id: String,
    /// effective id: the profile's when present, otherwise the vault's
    pub device_id: String,
    pub config: String,
    pub space: String,
    pub space_matches_config: bool,
    pub matches: bool,
    pub history: Vec<IdentityChange>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BaseIdentity {
    pub config: String,
    pub space: String,
    /// the active account whose stored pair equals the base pair
    pub owner: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceOverview {
    pub accounts: Vec<DeviceAccount>,
    pub base: BaseIdentity,
    pub conflicts: Vec<String>,
}

fn read_trim(path: &Path) -> io::Result<String> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(s.trim().to_string()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e),
    }
}

fn current_machine(base: &Path) -> (String, String) {
    let config = read_trim(&base.join("device_configuration_id")).unwrap_or_default();
    let space = read_trim(&base.join("device_space_id")).unwrap_or_default();
    (config, space)
}

fn write_text(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("aqm-tmp");
    fs::write(&tmp, text)?;
    fs::rename(tmp, path)
}

fn write_base(base: &Path, config: &str, space: &str) -> io::Result<()> {
    if config.is_empty() {
        let _ = fs::remove_file(base.join("device_configuration_id"));
    } else {
        write_text(&base.join("device_configuration_id"), config)?;
    }
    if space.is_empty() {
        let _ = fs::remove_file(base.join("device_space_id"));
    } else {
        write_text(&base.join("device_space_id"), space)?;
    }
    if read_trim(&base.join("device_configuration_id"))? != config {
        return Err(io::Error::other("安装 ID 写入后读回不一致"));
    }
    if read_trim(&base.join("device_space_id"))? != space {
        return Err(io::Error::other("space ID 写入后读回不一致"));
    }
    Ok(())
}

fn derived_ok(config: &str, space: &str) -> bool {
    config.is_empty() || space == device::space_id_for(config)
}

fn all_device_ids(base: &Path, vault_dir: &Path) -> io::Result<Vec<(String, String)>> {
    let manifest = profiles::read(base)?;
    let entries = vault::list(vault_dir)?;
    let mut pairs = Vec::new();
    for e in &manifest.entries {
        if let Some(id) = device::read(&device::profile_dir(base, e)) {
            pairs.push((e.id.clone(), id));
        }
    }
    for e in &entries {
        if !e.device_id.is_empty() {
            pairs.push((e.id.clone(), e.device_id.clone()));
        }
    }
    Ok(pairs)
}

pub fn used_device_ids(base: &Path, vault_dir: &Path, exclude: &str) -> io::Result<HashSet<String>> {
    Ok(all_device_ids(base, vault_dir)?
        .into_iter()
        .filter(|(account, _)| account != exclude)
        .map(|(_, id)| id)
        .collect())
}

/// The other account that already uses one of `id`'s device ids, if any.
pub fn conflict_for(base: &Path, vault_dir: &Path, id: &str) -> io::Result<Option<String>> {
    let pairs = all_device_ids(base, vault_dir)?;
    let mine: HashSet<String> = pairs
        .iter()
        .filter(|(account, _)| account == id)
        .map(|(_, id)| id.clone())
        .collect();
    if mine.is_empty() {
        return Ok(None);
    }
    for (account, other) in pairs {
        if account != id && mine.contains(&other) {
            return Ok(Some(account));
        }
    }
    Ok(None)
}

fn used_configs(vault_dir: &Path, exclude: &str) -> io::Result<HashSet<String>> {
    Ok(vault::list(vault_dir)?
        .into_iter()
        .filter(|e| e.id != exclude)
        .map(|e| e.device_configuration_id)
        .filter(|c| !c.is_empty())
        .collect())
}

pub fn overview(base: &Path, vault_dir: &Path, store: &dyn RawStore) -> io::Result<DeviceOverview> {
    let views = accounts::list(base, vault_dir, store)?;
    let manifest = profiles::read(base)?;
    let entries = vault::list(vault_dir)?;
    let (base_config, base_space) = current_machine(base);
    let active = manifest.last_active.clone();
    let mut owner = None;
    let mut accounts = Vec::new();
    let mut device_ids: HashMap<String, HashSet<String>> = HashMap::new();
    let mut configs: HashMap<String, HashSet<String>> = HashMap::new();
    let mut spaces: HashMap<String, HashSet<String>> = HashMap::new();
    for v in views {
        let profile = manifest
            .entries
            .iter()
            .find(|e| e.id == v.id)
            .and_then(|e| device::read(&device::profile_dir(base, e)));
        let entry: Option<&VaultEntry> = entries.iter().find(|e| e.id == v.id);
        let vault_id = entry.map(|e| e.device_id.clone()).unwrap_or_default();
        let device_id = profile.clone().unwrap_or_else(|| vault_id.clone());
        let config = entry.map(|e| e.device_configuration_id.clone()).unwrap_or_default();
        let space = entry.map(|e| e.device_space_id.clone()).unwrap_or_default();
        let matches = profile.as_deref().is_some_and(|p| !vault_id.is_empty() && p == vault_id);
        if !device_id.is_empty() {
            device_ids.entry(device_id.clone()).or_default().insert(v.id.clone());
        }
        if !config.is_empty() {
            configs.entry(config.clone()).or_default().insert(v.id.clone());
        }
        if !space.is_empty() {
            spaces.entry(space.clone()).or_default().insert(v.id.clone());
        }
        if active.as_deref() == Some(v.id.as_str())
            && !config.is_empty()
            && config == base_config
            && space == base_space
        {
            owner = Some(v.id.clone());
        }
        accounts.push(DeviceAccount {
            id: v.id,
            display_name: v.display_name,
            email: v.email,
            active: v.active,
            on_desktop: v.on_desktop,
            profile_device_id: profile,
            vault_device_id: vault_id,
            device_id,
            space_matches_config: derived_ok(&config, &space),
            config,
            space,
            matches,
            history: entry.map(|e| e.identity_history.clone()).unwrap_or_default(),
        });
    }
    let mut conflicts = Vec::new();
    for (id, accounts) in device_ids {
        if accounts.len() > 1 {
            conflicts.push(format!("设备 ID {id}"));
        }
    }
    for (id, accounts) in configs {
        if accounts.len() > 1 {
            conflicts.push(format!("安装 ID {id}"));
        }
    }
    for (id, accounts) in spaces {
        if accounts.len() > 1 {
            conflicts.push(format!("space {id}"));
        }
    }
    conflicts.sort();
    Ok(DeviceOverview {
        accounts,
        base: BaseIdentity { config: base_config, space: base_space, owner },
        conflicts,
    })
}

fn require_closed() -> Result<(), DeviceIdError> {
    if desktop::find()?.is_some() {
        return Err(DeviceIdError::DesktopRunning);
    }
    Ok(())
}

fn new_config(used: &HashSet<String>) -> (String, String) {
    loop {
        let config = device::new_id();
        if !used.contains(&config) {
            let space = device::space_id_for(&config);
            return (config, space);
        }
    }
}

/// Make sure an account has a complete, unique identity. `adopt_base` is for
/// the account that was already active: it owns the base pair, so it takes it
/// instead of generating a new one.
pub fn ensure_identity(
    base: &Path,
    vault_dir: &Path,
    id: &str,
    adopt_base: bool,
) -> Result<DeviceIdentity, DeviceIdError> {
    let entries = vault::list(vault_dir)?;
    let entry = entries
        .iter()
        .find(|e| e.id == id)
        .ok_or_else(|| DeviceIdError::NoAccount(id.into()))?
        .clone();
    let (base_config, base_space) = current_machine(base);
    let mut config = entry.device_configuration_id.clone();
    let mut space = entry.device_space_id.clone();

    if config.is_empty() {
        if adopt_base && !base_config.is_empty() {
            config = base_config;
            space = base_space;
        } else {
            let used = used_configs(vault_dir, id)?;
            let next = new_config(&used);
            config = next.0;
            space = next.1;
        }
    } else if space.is_empty() {
        space = device::space_id_for(&config);
    } else if !derived_ok(&config, &space) {
        return Err(DeviceIdError::ConfigDrift(id.into()));
    }

    if space != device::space_id_for(&config) {
        return Err(DeviceIdError::ConfigDrift(id.into()));
    }

    let used = used_configs(vault_dir, id)?;
    if used.contains(&config) {
        return Err(DeviceIdError::ConfigConflict(config));
    }
    for other in &entries {
        if other.id != id && !other.device_space_id.is_empty() && other.device_space_id == space {
            return Err(DeviceIdError::ConfigConflict(other.device_configuration_id.clone()));
        }
    }

    vault::set_identity(vault_dir, id, &entry.device_id, &config, &space)?;
    Ok(DeviceIdentity { device_id: entry.device_id, config, space })
}

/// Prepare the vault identities for a switch: the account being left adopts
/// the base pair if it has none, and the target gets a complete unique pair.
pub fn prepare_switch(
    base: &Path,
    vault_dir: &Path,
    target: &str,
    previous: Option<&str>,
) -> Result<(), DeviceIdError> {
    if let Some(prev) = previous {
        if prev != target {
            let _ = ensure_identity(base, vault_dir, prev, true);
        }
    }
    let adopt = previous.is_none() || previous == Some(target);
    ensure_identity(base, vault_dir, target, adopt)?;
    Ok(())
}

/// Project the account's config/space into the base files (before launch).
pub fn write_base_identity(base: &Path, vault_dir: &Path, id: &str) -> Result<(), DeviceIdError> {
    let entry = vault::list(vault_dir)?
        .into_iter()
        .find(|e| e.id == id)
        .ok_or_else(|| DeviceIdError::NoAccount(id.into()))?;
    if entry.device_configuration_id.is_empty() {
        return Err(DeviceIdError::ConfigDrift(id.into()));
    }
    if !derived_ok(&entry.device_configuration_id, &entry.device_space_id) {
        return Err(DeviceIdError::ConfigDrift(id.into()));
    }
    write_base(base, &entry.device_configuration_id, &entry.device_space_id)?;
    Ok(())
}

/// Rotate a complete identity. The old bundle goes to history. The profile
/// and (when this account is active) the base files are written now.
pub fn replace_account(base: &Path, vault_dir: &Path, id: &str) -> Result<DeviceIdentity, DeviceIdError> {
    require_closed()?;
    let manifest = profiles::read(base)?;
    let entry = vault::list(vault_dir)?
        .into_iter()
        .find(|e| e.id == id)
        .ok_or_else(|| DeviceIdError::NoAccount(id.into()))?;
    let profile = manifest.entries.iter().find(|e| e.id == id).cloned();
    let active = manifest.last_active.as_deref() == Some(id);

    // archive what the profile/base are really using, not a stale vault copy
    let mut old_device = entry.device_id.clone();
    if let Some(p) = &profile {
        if let Some(current) = device::read(&device::profile_dir(base, p)) {
            old_device = current;
        }
    }
    if active {
        if entry.device_configuration_id.is_empty() {
            let (base_config, base_space) = current_machine(base);
            if !base_config.is_empty() {
                vault::set_identity(vault_dir, id, &old_device, &base_config, &base_space)?;
            }
        }
    }

    let used = used_device_ids(base, vault_dir, id)?;
    let mut next_device = device::new_id();
    while used.contains(&next_device) {
        next_device = device::new_id();
    }
    let configs = used_configs(vault_dir, id)?;
    let (next_config, next_space) = new_config(&configs);

    if let Some(p) = &profile {
        let dir = device::profile_dir(base, p);
        device::write_id(&dir, &next_device)?;
        if device::read(&dir).as_deref() != Some(next_device.as_str()) {
            return Err(DeviceIdError::Io(io::Error::other("设备 ID 写入后读回不一致")));
        }
    }
    if active {
        write_base(base, &next_config, &next_space)?;
    }
    vault::record_identity_change(
        vault_dir,
        id,
        &next_device,
        &next_config,
        &next_space,
        &chrono::Utc::now().to_rfc3339(),
    )?;
    Ok(DeviceIdentity { device_id: next_device, config: next_config, space: next_space })
}

/// Restore one archived identity; the current bundle is archived in turn.
pub fn restore_account(base: &Path, vault_dir: &Path, id: &str, target: &IdentityChange) -> Result<(), DeviceIdError> {
    require_closed()?;
    let manifest = profiles::read(base)?;
    let entry = vault::list(vault_dir)?
        .into_iter()
        .find(|e| e.id == id)
        .ok_or_else(|| DeviceIdError::NoAccount(id.into()))?;
    if !entry.identity_history.iter().any(|h| {
        h.device_id == target.device_id && h.config == target.config && h.space == target.space
    }) {
        return Err(DeviceIdError::NotInHistory);
    }
    if !target.config.is_empty() && !derived_ok(&target.config, &target.space) {
        return Err(DeviceIdError::ConfigDrift(id.into()));
    }

    let used = used_device_ids(base, vault_dir, id)?;
    if !target.device_id.is_empty() && used.contains(&target.device_id) {
        return Err(DeviceIdError::Conflict(target.device_id.clone()));
    }
    let configs = used_configs(vault_dir, id)?;
    if !target.config.is_empty() && configs.contains(&target.config) {
        return Err(DeviceIdError::ConfigConflict(target.config.clone()));
    }

    let profile = manifest.entries.iter().find(|e| e.id == id);
    let active = manifest.last_active.as_deref() == Some(id);
    if let Some(p) = profile {
        let dir = device::profile_dir(base, p);
        device::write_id(&dir, &target.device_id)?;
        if device::read(&dir).as_deref() != Some(target.device_id.as_str()) {
            return Err(DeviceIdError::Io(io::Error::other("设备 ID 写入后读回不一致")));
        }
    }
    if active {
        write_base(base, &target.config, &target.space)?;
    }
    vault::record_identity_change(
        vault_dir,
        id,
        &target.device_id,
        &target.config,
        &target.space,
        &chrono::Utc::now().to_rfc3339(),
    )?;
    Ok(())
}
