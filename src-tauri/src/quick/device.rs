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
                current,
            ));
        }
        return Ok(current);
    }
    if let Some(k) = known.filter(|s| !s.is_empty()) {
        if used.contains(k) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                k.to_string(),
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
    super::fsutil::atomic_write(&dir.join(DEVICE_FILE), id.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aqm-device-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn new_ids_are_v4_uuids() {
        let id = new_id();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
        assert_ne!(new_id(), id);
    }

    #[test]
    fn space_id_is_a_stable_uuid5_in_32_hex() {
        let a = space_id_for("cfg-1");
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(&a[12..13], "5");
        assert_eq!(a, space_id_for("cfg-1"));
        assert_ne!(a, space_id_for("cfg-2"));
    }

    #[test]
    fn profile_dir_follows_data_path() {
        let base = Path::new("/base");
        let e = |p: &str| ProfileEntry { id: "social-a".into(), data_path: p.into(), ..Default::default() };
        assert_eq!(profile_dir(base, &e("")), base.join("profiles").join("social-a"));
        assert_eq!(profile_dir(base, &e("profiles/x")), base.join("profiles/x"));
    }

    #[test]
    fn ensure_unique_keeps_the_file_reuses_a_free_id_and_refuses_a_taken_one() {
        let dir = temp("unique");
        let mut used = HashSet::new();
        used.insert("taken".to_string());

        // no file: a free known id is written
        assert_eq!(ensure_unique(&dir, Some("mine"), &used).unwrap(), "mine");
        assert_eq!(read(&dir).as_deref(), Some("mine"));
        // the file wins over a different known id
        assert_eq!(ensure_unique(&dir, Some("other"), &used).unwrap(), "mine");
        // a file holding an id another account uses is refused, not rotated
        write_id(&dir, "taken").unwrap();
        assert_eq!(ensure_unique(&dir, None, &used).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        // a known id that is taken is refused
        let fresh = temp("unique2");
        assert!(ensure_unique(&fresh, Some("taken"), &used).is_err());
        // nothing known: a new id
        let id = ensure_unique(&fresh, None, &used).unwrap();
        assert_eq!(id.len(), 36);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&fresh);
    }
}
