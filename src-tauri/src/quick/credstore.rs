//! The desktop app's session credentials in the OS store.
//!
//! The app writes them with python-keyring: service `quickwork-<provider>-<id>`,
//! user `session`. On Windows a value over the 2560-byte CredWrite cap is split
//! into `<target>_chunk_<n>` entries behind a `{"__chunks__": n, "__sha__": …}`
//! manifest, and the same value is kept again under `session@<target>`.

use serde_json::Value;
use sha2::{Digest, Sha256};

pub const USER: &str = "session";
/// UTF-16 code units per chunk: 2400 bytes, under the 2560-byte cap.
const CHUNK_UNITS: usize = 1200;
const MAX_STALE_CHUNKS: usize = 16;

#[derive(Debug, thiserror::Error)]
pub enum CredError {
    #[error("credential store: {0}")]
    Os(String),
    #[error("chunked credential {0} is incomplete or its checksum doesn't match")]
    Corrupt(String),
    #[allow(dead_code)]
    #[error("credential store isn't supported on this platform")]
    Unsupported,
}

/// Low-level store: one opaque string per target.
pub trait RawStore {
    fn get(&self, target: &str) -> Result<Option<String>, CredError>;
    fn set(&self, target: &str, value: &str) -> Result<(), CredError>;
    fn delete(&self, target: &str) -> Result<(), CredError>;
}

fn sha_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

/// Read a value, following a chunk manifest if there is one.
pub fn read_value(store: &dyn RawStore, target: &str) -> Result<Option<String>, CredError> {
    let Some(raw) = store.get(target)? else { return Ok(None) };
    let manifest = serde_json::from_str::<Value>(&raw).ok();
    let chunks = manifest.as_ref().and_then(|m| m.get("__chunks__")).and_then(Value::as_u64);
    let Some(n) = chunks.filter(|n| *n > 0) else { return Ok(Some(raw)) };
    let mut full = String::new();
    for i in 0..n {
        match store.get(&format!("{target}_chunk_{i}"))? {
            Some(part) => full.push_str(&part),
            None => return Err(CredError::Corrupt(target.into())),
        }
    }
    if let Some(sha) = manifest.as_ref().and_then(|m| m.get("__sha__")).and_then(Value::as_str) {
        // python-keyring hashes UTF-16LE bytes on Windows, UTF-8 elsewhere
        if !sha.is_empty() && sha != sha_hex(full.as_bytes()) && sha != sha_hex(&utf16le(&full)) {
            return Err(CredError::Corrupt(target.into()));
        }
    }
    Ok(Some(full))
}

/// Split `value` into chunks of at most `units` UTF-16 code units, never
/// between the halves of a surrogate pair.
fn split_utf16(value: &str, units: usize) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut len = 0;
    for ch in value.chars() {
        let w = ch.len_utf16();
        if len + w > units && !cur.is_empty() {
            parts.push(std::mem::take(&mut cur));
            len = 0;
        }
        cur.push(ch);
        len += w;
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}

/// The entries a chunked write puts in the store, manifest first.
pub fn chunk_entries(target: &str, value: &str) -> Vec<(String, String)> {
    let parts = split_utf16(value, CHUNK_UNITS);
    let manifest = format!(r#"{{"__chunks__": {}, "__sha__": "{}"}}"#, parts.len(), sha_hex(value.as_bytes()));
    let mut out = vec![(target.to_string(), manifest)];
    out.extend(parts.into_iter().enumerate().map(|(i, p)| (format!("{target}_chunk_{i}"), p)));
    out
}

/// Write a value the way the desktop reads it back. `chunked` is the
/// Windows layout; elsewhere the value is one entry.
pub fn write_value(store: &dyn RawStore, target: &str, value: &str, chunked: bool) -> Result<(), CredError> {
    if !chunked {
        return store.set(target, value);
    }
    let entries = chunk_entries(target, value);
    let parts = entries.len() - 1;
    for (t, v) in &entries {
        store.set(t, v)?;
    }
    for i in parts..MAX_STALE_CHUNKS {
        let _ = store.delete(&format!("{target}_chunk_{i}"));
    }
    Ok(())
}

pub fn delete_value(store: &dyn RawStore, target: &str) -> Result<(), CredError> {
    store.delete(target)?;
    for i in 0..MAX_STALE_CHUNKS {
        let _ = store.delete(&format!("{target}_chunk_{i}"));
    }
    Ok(())
}

/// The account's session JSON, from the plain target or its `session@` twin.
pub fn read_session(store: &dyn RawStore, target: &str) -> Result<Option<Value>, CredError> {
    for t in [target.to_string(), format!("{USER}@{target}")] {
        if let Some(text) = read_value(store, &t)? {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                return Ok(Some(v));
            }
        }
    }
    Ok(None)
}

/// Write the session to the target and, on Windows, its `session@` twin too.
#[allow(dead_code)]
pub fn write_session(store: &dyn RawStore, target: &str, session: &Value) -> Result<(), CredError> {
    let text = session.to_string();
    let chunked = cfg!(windows);
    write_value(store, target, &text, chunked)?;
    if chunked {
        write_value(store, &format!("{USER}@{target}"), &text, true)?;
    }
    Ok(())
}

#[allow(dead_code)]
pub fn delete_session(store: &dyn RawStore, target: &str) -> Result<(), CredError> {
    delete_value(store, target)?;
    if cfg!(windows) {
        let _ = delete_value(store, &format!("{USER}@{target}"));
    }
    Ok(())
}

/// The OS store for this platform.
pub fn system() -> Box<dyn RawStore> {
    #[cfg(windows)]
    return Box::new(windows_store::WinCred);
    #[cfg(target_os = "macos")]
    return Box::new(mac_store::Keychain);
    #[cfg(not(any(windows, target_os = "macos")))]
    return Box::new(Unsupported);
}

#[cfg(not(any(windows, target_os = "macos")))]
struct Unsupported;
#[cfg(not(any(windows, target_os = "macos")))]
impl RawStore for Unsupported {
    fn get(&self, _: &str) -> Result<Option<String>, CredError> {
        Err(CredError::Unsupported)
    }
    fn set(&self, _: &str, _: &str) -> Result<(), CredError> {
        Err(CredError::Unsupported)
    }
    fn delete(&self, _: &str) -> Result<(), CredError> {
        Err(CredError::Unsupported)
    }
}

#[cfg(windows)]
mod windows_store {
    use super::{CredError, RawStore, USER};
    use windows::Win32::Foundation::ERROR_NOT_FOUND;
    use windows::Win32::Security::Credentials::{
        CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW, CredWriteW,
    };
    use windows::core::{HSTRING, PWSTR};

    pub struct WinCred;

    fn not_found(e: &windows::core::Error) -> bool {
        e.code() == ERROR_NOT_FOUND.to_hresult()
    }

    impl RawStore for WinCred {
        fn get(&self, target: &str) -> Result<Option<String>, CredError> {
            let name = HSTRING::from(target);
            let mut cred: *mut CREDENTIALW = std::ptr::null_mut();
            match unsafe { CredReadW(&name, CRED_TYPE_GENERIC, None, &mut cred) } {
                Err(e) if not_found(&e) => Ok(None),
                Err(e) => Err(CredError::Os(e.message())),
                Ok(()) => unsafe {
                    let c = &*cred;
                    let bytes = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize);
                    let units: Vec<u16> = bytes.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
                    CredFree(cred as *const _);
                    Ok(Some(String::from_utf16_lossy(&units)))
                },
            }
        }

        fn set(&self, target: &str, value: &str) -> Result<(), CredError> {
            let mut blob = super::utf16le(value);
            if blob.len() > 2560 {
                return Err(CredError::Os(format!("{target}: {} bytes is over the 2560-byte limit", blob.len())));
            }
            let mut name: Vec<u16> = target.encode_utf16().chain([0]).collect();
            let mut user: Vec<u16> = USER.encode_utf16().chain([0]).collect();
            let cred = CREDENTIALW {
                Type: CRED_TYPE_GENERIC,
                TargetName: PWSTR(name.as_mut_ptr()),
                UserName: PWSTR(user.as_mut_ptr()),
                CredentialBlobSize: blob.len() as u32,
                CredentialBlob: blob.as_mut_ptr(),
                Persist: CRED_PERSIST_LOCAL_MACHINE,
                ..Default::default()
            };
            unsafe { CredWriteW(&cred, 0) }.map_err(|e| CredError::Os(e.message()))
        }

        fn delete(&self, target: &str) -> Result<(), CredError> {
            let name = HSTRING::from(target);
            match unsafe { CredDeleteW(&name, CRED_TYPE_GENERIC, None) } {
                Err(e) if not_found(&e) => Ok(()),
                r => r.map_err(|e| CredError::Os(e.message())),
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod mac_store {
    use super::{CredError, RawStore, USER};
    use security_framework::passwords::{delete_generic_password, get_generic_password, set_generic_password};

    pub struct Keychain;

    // errSecItemNotFound
    const NOT_FOUND: i32 = -25300;

    impl RawStore for Keychain {
        fn get(&self, target: &str) -> Result<Option<String>, CredError> {
            match get_generic_password(target, USER) {
                Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
                Err(e) if e.code() == NOT_FOUND => Ok(None),
                Err(e) => Err(CredError::Os(e.to_string())),
            }
        }

        fn set(&self, target: &str, value: &str) -> Result<(), CredError> {
            set_generic_password(target, USER, value.as_bytes()).map_err(|e| CredError::Os(e.to_string()))
        }

        fn delete(&self, target: &str) -> Result<(), CredError> {
            match delete_generic_password(target, USER) {
                Err(e) if e.code() != NOT_FOUND => Err(CredError::Os(e.to_string())),
                _ => Ok(()),
            }
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    #[derive(Default)]
    pub struct MemStore(pub RefCell<BTreeMap<String, String>>);
    impl RawStore for MemStore {
        fn get(&self, t: &str) -> Result<Option<String>, CredError> {
            Ok(self.0.borrow().get(t).cloned())
        }
        fn set(&self, t: &str, v: &str) -> Result<(), CredError> {
            self.0.borrow_mut().insert(t.into(), v.into());
            Ok(())
        }
        fn delete(&self, t: &str) -> Result<(), CredError> {
            self.0.borrow_mut().remove(t);
            Ok(())
        }
    }

    #[test]
    fn chunks_stay_under_the_blob_cap_and_roundtrip() {
        let value = "x".repeat(5000) + "\u{1F600}" + &"y".repeat(1199);
        let s = MemStore::default();
        write_value(&s, "t", &value, true).unwrap();
        for (k, v) in s.0.borrow().iter() {
            assert!(utf16le(v).len() <= 2560, "{k} too big");
        }
        assert_eq!(read_value(&s, "t").unwrap().as_deref(), Some(value.as_str()));
    }

    #[test]
    fn surrogate_pairs_are_never_split() {
        let smile = "\u{1F600}";
        let parts = split_utf16(&smile.repeat(1000), CHUNK_UNITS);
        assert!(parts.iter().all(|p| p.chars().all(|c| c == '\u{1F600}')));
        assert_eq!(parts.concat(), smile.repeat(1000));
    }

    #[test]
    fn rewrite_with_fewer_chunks_drops_stale_ones() {
        let s = MemStore::default();
        write_value(&s, "t", &"a".repeat(4000), true).unwrap();
        write_value(&s, "t", "short", true).unwrap();
        assert!(s.0.borrow().get("t_chunk_1").is_none());
        assert_eq!(read_value(&s, "t").unwrap().as_deref(), Some("short"));
    }

    #[test]
    fn utf16_checksum_from_python_keyring_is_accepted() {
        let s = MemStore::default();
        let full = "hello";
        s.set("t", &format!(r#"{{"__chunks__": 1, "__sha__": "{}"}}"#, sha_hex(&utf16le(full)))).unwrap();
        s.set("t_chunk_0", full).unwrap();
        assert_eq!(read_value(&s, "t").unwrap().as_deref(), Some(full));
    }

    #[test]
    fn bad_checksum_or_missing_chunk_is_corrupt() {
        let s = MemStore::default();
        s.set("t", r#"{"__chunks__": 2, "__sha__": ""}"#).unwrap();
        s.set("t_chunk_0", "a").unwrap();
        assert!(matches!(read_value(&s, "t"), Err(CredError::Corrupt(_))));
        s.set("t_chunk_1", "b").unwrap();
        s.set("t", r#"{"__chunks__": 2, "__sha__": "00"}"#).unwrap();
        assert!(matches!(read_value(&s, "t"), Err(CredError::Corrupt(_))));
    }

    #[test]
    fn session_falls_back_to_the_twin() {
        let s = MemStore::default();
        s.set("session@t", r#"{"refresh_token":"r"}"#).unwrap();
        assert_eq!(read_session(&s, "t").unwrap().unwrap()["refresh_token"], "r");
    }
}
