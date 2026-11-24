//! Small file helpers shared by everything that rewrites a file in place.
//!
//! - [`atomic_write`]: write to a temp file nobody else uses, flush it, then
//!   rename it over the target. A reader sees the old file or the new one,
//!   never half of one, and two writers never share a temp file.
//! - [`FileLock`]: an exclusive OS file lock (`File::lock`). It serialises
//!   threads of this app and other processes (the `aqm` command line), and
//!   the OS drops it if the process dies, so there are no stale locks.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

/// A temp name next to `path` that no other writer, thread or process uses.
fn temp_path(path: &Path) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!(".{name}.{}-{n}.tmp", std::process::id()))
}

/// Replace `path` with `bytes` atomically.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = temp_path(path);
    let result = (|| {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// An exclusive lock on a lock file, released when dropped.
#[derive(Debug)]
pub struct FileLock {
    _file: File,
}

fn open_lock(path: &Path) -> io::Result<File> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    OpenOptions::new().create(true).read(true).write(true).truncate(false).open(path)
}

impl FileLock {
    /// Wait until the lock is free and take it.
    pub fn acquire(path: &Path) -> io::Result<Self> {
        let file = open_lock(path)?;
        file.lock()?;
        Ok(Self { _file: file })
    }

    /// Take the lock, or `None` when someone else holds it.
    pub fn try_acquire(path: &Path) -> io::Result<Option<Self>> {
        let file = open_lock(path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(e)) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aqm-fsutil-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect()
    }

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp_files() {
        let dir = temp("write");
        let path = dir.join("a.json");
        atomic_write(&path, b"one").unwrap();
        atomic_write(&path, b"two").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        assert!(leftovers(&dir).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn temp_names_never_repeat() {
        let p = Path::new("x/accounts.json");
        assert_ne!(temp_path(p), temp_path(p));
    }

    #[test]
    fn a_held_lock_is_busy_until_dropped() {
        let dir = temp("lock");
        let path = dir.join("op.lock");
        let first = FileLock::try_acquire(&path).unwrap();
        assert!(first.is_some());
        assert!(FileLock::try_acquire(&path).unwrap().is_none());
        drop(first);
        assert!(FileLock::try_acquire(&path).unwrap().is_some());
        let _ = fs::remove_dir_all(&dir);
    }
}
