//! One Pigeon per data folder (PER-5), and test-instance mode (sprint 2,
//! W1). The first Pigeon holds an exclusive lock on `<data dir>/pigeon.lock`
//! with its pid in it; a second one on the same folder hands over to it
//! (`native::hand_over`) and exits.

use std::{
    fs::{File, OpenOptions, TryLockError},
    io::{Read as _, Seek as _, Write as _},
    path::Path,
};

/// `PIGEON_DEV=1`: a tester's copy. No global shortcuts, no login item,
/// and notification tags get a prefix, so it can't clash with the
/// installed Pigeon.
pub fn dev_mode() -> bool {
    std::env::var("PIGEON_DEV").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// Whether global shortcuts may be registered (W7b).
pub fn global_shortcuts_allowed() -> bool {
    !dev_mode()
}

/// Held for as long as the process runs.
pub struct InstanceLock {
    _file: File,
}

/// Another Pigeon has the folder: its pid, when it wrote one.
#[derive(Debug, PartialEq, Eq)]
pub struct Running {
    pub pid: Option<i32>,
}

/// Take `dir`'s lock, or say who has it. A lock that can't be made at all
/// (a read-only folder, say) doesn't stop Pigeon.
pub fn acquire(dir: &Path) -> Result<Option<InstanceLock>, Running> {
    let _ = std::fs::create_dir_all(dir);
    let Ok(mut file) = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("pigeon.lock"))
    else {
        return Ok(None);
    };
    match file.try_lock() {
        Ok(()) => {
            let _ = file.set_len(0);
            let _ = file.rewind();
            let _ = write!(file, "{}", std::process::id());
            let _ = file.flush();
            Ok(Some(InstanceLock { _file: file }))
        }
        Err(TryLockError::WouldBlock) => {
            let mut pid = String::new();
            let _ = file.read_to_string(&mut pid);
            Err(Running {
                pid: pid.trim().parse().ok(),
            })
        }
        Err(TryLockError::Error(_)) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_lock_on_the_folder_finds_the_first() {
        let dir = std::env::temp_dir().join(format!("pigeon-lock-{}", std::process::id()));
        let first = acquire(&dir).unwrap();
        assert!(first.is_some());
        assert_eq!(
            acquire(&dir).err(),
            Some(Running {
                pid: Some(std::process::id() as i32)
            })
        );
        // Another folder is another Pigeon.
        let other = dir.join("other");
        assert!(acquire(&other).unwrap().is_some());
        // Once the first goes, the folder is free again.
        drop(first);
        assert!(acquire(&dir).unwrap().is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
