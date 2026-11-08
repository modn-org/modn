use anyhow::{bail, Context, Result};
use fs2::FileExt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

/// Total time a caller waits for a contended bucket lock before it reports the
/// operation as already running.
///
/// A forked child on Linux inherits the lock's open file description until it
/// calls `exec`, so a fresh lock attempt can briefly see the previous owner's
/// lock after that owner released it. Retrying absorbs that window.
pub(super) const LOCK_WAIT: Duration = Duration::from_secs(2);

const RETRY_INTERVAL: Duration = Duration::from_millis(25);

/// Open a regular lock file and acquire its exclusive lock.
///
/// Only lock contention is retried, and only until `wait` elapses. Other I/O
/// errors fail at once. The returned file keeps the lock held until it drops.
pub(super) fn acquire_lock(
    path: &Path,
    unsafe_message: &'static str,
    busy_message: &'static str,
    wait: Duration,
) -> Result<File> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if !meta.is_file() || meta.file_type().is_symlink() {
            bail!("{unsafe_message}");
        }
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)?;
    let deadline = Instant::now() + wait;
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(file),
            Err(error) if is_contention(&error) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(error).context(busy_message);
                }
                thread::sleep(remaining.min(RETRY_INTERVAL));
            }
            Err(error) => return Err(error).context(busy_message),
        }
    }
}

fn is_contention(error: &io::Error) -> bool {
    if error.kind() == ErrorKind::WouldBlock {
        return true;
    }
    // Windows reports ERROR_LOCK_VIOLATION, whose `ErrorKind` varies by toolchain.
    error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
}

#[cfg(test)]
mod tests {
    use super::{acquire_lock, LOCK_WAIT};
    use fs2::FileExt;
    use std::fs::{self, OpenOptions};
    use std::thread;
    use std::time::Duration;

    const SHORT_WAIT: Duration = Duration::from_millis(600);

    fn open(path: &std::path::Path) -> fs::File {
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .unwrap()
    }

    #[test]
    fn waits_for_a_short_hold_and_acquires_after_release() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("lock");
        let held = open(&path);
        FileExt::lock_exclusive(&held).unwrap();
        let release = thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            drop(held);
        });

        let lock = acquire_lock(
            &path,
            "unsafe test lock",
            "test lock is already running",
            LOCK_WAIT,
        )
        .unwrap();
        release.join().unwrap();
        let contender = open(&path);
        assert!(contender.try_lock_exclusive().is_err());
        drop(lock);
    }

    #[test]
    fn reports_contention_after_the_wait() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("lock");
        let held = open(&path);
        FileExt::lock_exclusive(&held).unwrap();

        let error = acquire_lock(
            &path,
            "unsafe test lock",
            "test lock is already running",
            SHORT_WAIT,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "test lock is already running");
        drop(held);
        acquire_lock(
            &path,
            "unsafe test lock",
            "test lock is already running",
            SHORT_WAIT,
        )
        .unwrap();
    }
}
