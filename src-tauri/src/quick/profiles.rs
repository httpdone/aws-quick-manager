//! The desktop app's profile manifest, `<base>/profiles.json`.
//!
//! Each signed-in account is one entry; `last_active` names the one the app
//! starts with. Unknown fields are kept as-is so a rewrite never drops what a
//! newer desktop version added.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const MANIFEST_FILE: &str = "profiles.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default)]
    pub entries: Vec<ProfileEntry>,
    #[serde(default)]
    pub last_active: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ProfileEntry {
    pub id: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub data_path: String,
    #[serde(default)]
    pub login_metadata: Map<String, Value>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub last_used_at: Option<String>,
    #[serde(default)]
    pub refresh_failing_since: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `~/.quickwork`, or `QUICKWORK_BASE` when set (the app honors it too).
pub fn base_dir() -> PathBuf {
    match std::env::var("QUICKWORK_BASE") {
        Ok(v) if !v.trim().is_empty() => PathBuf::from(v.trim()),
        _ => dirs::home_dir().unwrap_or_default().join(".quickwork"),
    }
}

/// The desktop's profile id for a social account: `social-` plus the first
/// 12 hex digits of sha256(user-info `external_id`).
pub fn profile_id(provider: &str, external_id: &str) -> String {
    let digest = Sha256::digest(external_id.as_bytes());
    format!("{provider}-{}", &hex::encode(digest)[..12])
}

/// Credential-store target the desktop keeps an account's session under.
pub fn credential_target(entry: &ProfileEntry) -> String {
    let provider = if entry.provider.is_empty() { "social" } else { &entry.provider };
    format!("quickwork-{provider}-{}", entry.id)
}

/// Read the manifest; a missing file is an empty manifest.
pub fn read(base: &Path) -> io::Result<Manifest> {
    match fs::read_to_string(base.join(MANIFEST_FILE)) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Manifest { version: 1, ..Default::default() }),
        Err(e) => Err(e),
    }
}

/// Write the manifest atomically (temp file + rename).
#[allow(dead_code)]
pub fn write(base: &Path, manifest: &Manifest) -> io::Result<()> {
    let text = serde_json::to_string_pretty(manifest).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    super::fsutil::atomic_write(&base.join(MANIFEST_FILE), (text + "\n").as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_id_is_sha256_prefix() {
        // sha256("abc") = ba7816bf8f01cfea…
        assert_eq!(profile_id("social", "abc"), "social-ba7816bf8f01");
    }

    #[test]
    fn credential_target_matches_desktop_naming() {
        let e = ProfileEntry { id: "social-ba7816bf8f01".into(), provider: "social".into(), ..Default::default() };
        assert_eq!(credential_target(&e), "quickwork-social-social-ba7816bf8f01");
    }

    #[test]
    fn manifest_roundtrip_keeps_unknown_fields() {
        let text = r#"{"version":1,"entries":[{"id":"social-x","provider":"social","display_name":"A","data_path":"profiles/social-x","login_metadata":{"issuer_url":"i"},"created_at":"t","last_used_at":"t","refresh_failing_since":null,"future":7}],"last_active":"social-x","later":true}"#;
        let m: Manifest = serde_json::from_str(text).unwrap();
        assert_eq!(m.last_active.as_deref(), Some("social-x"));
        assert_eq!(m.entries[0].extra["future"], 7);
        let back: Value = serde_json::to_value(&m).unwrap();
        assert_eq!(back["later"], true);
        assert_eq!(back["entries"][0]["future"], 7);
    }

    #[test]
    fn read_write_roundtrip_in_temp_dir() {
        let dir = std::env::temp_dir().join(format!("aqm-profiles-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert!(read(&dir).unwrap().entries.is_empty());
        let mut m = Manifest { version: 1, ..Default::default() };
        m.entries.push(ProfileEntry { id: "social-a".into(), ..Default::default() });
        m.last_active = Some("social-a".into());
        write(&dir, &m).unwrap();
        assert_eq!(read(&dir).unwrap(), m);
        let _ = fs::remove_dir_all(&dir);
    }
}
