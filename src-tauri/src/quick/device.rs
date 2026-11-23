//! Per-account device identity.
//!
//! Each profile directory carries a `device_id`. The Quick backend returns
//! that file's contents from `/local/outpost-id`, and the renderer sends it
//! to the server as the device id (verified against the bundled outpost:
//! rewriting the file changes the reported id). Keeping one stable id per
//! account is therefore what keeps accounts separate on the wire.
//!
//! The desktop generates the file for a profile it creates. This module only
//! fills a gap and never rotates an id that is already in use: an existing
//! file always wins and is copied back into the vault.

use super::profiles::ProfileEntry;
use sha1::{Digest, Sha1};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const DEVICE_FILE: &str = "device_id";
/// Frozen prefix the agent uses to derive the device space id from the
/// install-wide configuration id. Must match the bundled agent exactly.
pub const SPACE_SEED_PREFIX: &str = "test-qw-device-";
const NAMESPACE_URL: [u8; 16] = [
    0x6b, 0xa7, 0xb8, 0x11, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30, 0xc8,
];

/// The profile's data directory; `data_path` is relative to the base.
pub fn profile_dir(base: &Path, entry: &ProfileEntry) -> PathBuf {
    let path = Path::new(&entry.data_path);
    if path.is_absolute() {
        path.to_path_buf()
    } else if entry.data_path.is_empty() {
        base.join("profiles").join(&entry.id)
    } else {
        base.join(path)
    }
}

/// The id the profile currently carries, if any.
pub fn read(dir: &Path) -> Option<String> {
    fs::read_to_string(dir.join(DEVICE_FILE))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// A fresh v4 UUID in the same shape the desktop writes.
pub fn new_id() -> String {
    let mut b: [u8; 16] = rand::random();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex::encode(b);
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

/// `uuid5(NAMESPACE_URL, "test-qw-device-<config>")`, the device space id the
/// agent derives from the install-wide configuration id.
pub fn space_id_for(config: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(NAMESPACE_URL);
    hasher.update(format!("{SPACE_SEED_PREFIX}{config}").as_bytes());
    let digest = hasher.finalize();
    let mut b = [0u8; 16];
    b.copy_from_slice(&digest[..16]);
    b[6] = (b[6] & 0x0f) | 0x50;
    b[8] = (b[8] & 0x3f) | 0x80;
    // the file and the agent use the 32-hex form, not the hyphenated UUID
    hex::encode(b)
}

/// Overwrite a profile's `device_id` with a specific value.
pub fn write_id(dir: &Path, id: &str) -> io::Result<()> {
    write(dir, id)
}

/// Fill a missing id without reusing one another account already has: an
/// existing file wins, a known vault id is reused only when it is free, and
/// otherwise a new id is generated.
pub fn ensure_unique(dir: &Path, known: Option<&str>, used: &HashSet<String>) -> io::Result<String> {
    if let Some(current) = read(dir) {
        if used.contains(&current) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("设备 ID {current} 已被其他账号使用"),
            ));
        }
        return Ok(current);
    }
    if let Some(k) = known.filter(|s| !s.is_empty()) {
        if used.contains(k) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("设备 ID {k} 已被其他账号使用"),
            ));
        }
        write(dir, k)?;
        return Ok(k.to_string());
    }
    let mut id = new_id();
    while used.contains(&id) {
        id = new_id();
    }
    write(dir, &id)?;
    Ok(id)
}

fn write(dir: &Path, id: &str) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{DEVICE_FILE}.tmp"));
    fs::write(&tmp, id)?;
    fs::rename(tmp, dir.join(DEVICE_FILE))
}
