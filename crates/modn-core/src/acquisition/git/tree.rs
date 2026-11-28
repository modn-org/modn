//! One Git process tree that Modn owns and confirms stopped.
//!
//! Git can start helpers, such as `git-remote-https`, that outlive the Git
//! process and keep writing to staging. Every fetch or clone therefore runs
//! inside its own tree: a Windows job object with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, or a Unix process group that the Git
//! child leads.
//!
//! [`TreeGuard`] owns that tree from the moment the child starts. Its `Drop`
//! stops the tree on an early return or a panic, so a caller cannot leave a
//! helper behind. A stop is only successful after the operating system
//! confirms that no process of the tree is alive.

use std::fmt;
use std::io;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as platform;
#[cfg(windows)]
use windows as platform;

/// Longest wait for a stopped tree to report that it is empty.
pub(crate) const STOP_TIMEOUT: Duration = Duration::from_secs(5);

/// How often the confirmation polls the operating system.
pub(crate) const STOP_POLL: Duration = Duration::from_millis(20);

/// Why Modn could not confine or stop one Git process tree.
#[derive(Debug)]
pub(crate) enum TreeError {
    /// The operating system refused a call or reported an error.
    Io(io::Error),
    /// The tree still had a live process after [`STOP_TIMEOUT`].
    Unconfirmed,
}

impl fmt::Display for TreeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(
                formatter,
                "the operating system refused a process-tree call: {error}"
            ),
            Self::Unconfirmed => write!(
                formatter,
                "a Git process stayed alive {} ms after the tree was stopped",
                STOP_TIMEOUT.as_millis()
            ),
        }
    }
}

impl std::error::Error for TreeError {}

/// Prepare `command` to start inside a new tree.
///
/// This must run before the spawn. A failure means the caller must not start
/// Git unconfined.
pub(crate) fn confine(command: &mut Command) -> Result<(), TreeError> {
    platform::Tree::confine(command).map_err(TreeError::Io)
}

/// The tree of one started Git child, stopped on `Drop` unless disarmed.
pub(crate) struct TreeGuard {
    tree: platform::Tree,
    stopped: bool,
}

impl TreeGuard {
    /// Adopt the tree that `child` started in.
    ///
    /// On Windows this assigns the suspended child to a new job and resumes
    /// it. On Unix it records the process group that the child leads.
    pub(crate) fn attach(child: &Child) -> Result<Self, TreeError> {
        Ok(Self {
            tree: platform::Tree::attach(child)?,
            stopped: false,
        })
    }

    /// Stop the tree, reap the direct child and confirm that it is empty.
    pub(crate) fn stop(&mut self, child: &mut Child) -> Result<(), TreeError> {
        if self.stopped {
            return Ok(());
        }
        self.tree.stop(child)?;
        self.confirm()
    }

    /// Stop the remaining tree after the direct child exited on its own.
    ///
    /// The wait that observed the exit already reaped the child. A helper can
    /// still be alive, so the same confirmation runs here.
    pub(crate) fn finish(&mut self) -> Result<(), TreeError> {
        if self.stopped {
            return Ok(());
        }
        self.tree.finish()?;
        self.confirm()
    }

    fn confirm(&mut self) -> Result<(), TreeError> {
        #[cfg(test)]
        if forced_unconfirmed() {
            return Err(TreeError::Unconfirmed);
        }
        self.stopped = true;
        Ok(())
    }
}

impl Drop for TreeGuard {
    fn drop(&mut self) {
        if !self.stopped {
            self.tree.abandon();
            self.stopped = true;
        }
    }
}

/// A bounded wait for `empty` to confirm that the tree is gone.
fn wait_until_empty(mut empty: impl FnMut() -> Result<bool, io::Error>) -> Result<(), TreeError> {
    let deadline = Instant::now() + STOP_TIMEOUT;
    loop {
        if empty().map_err(TreeError::Io)? {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(TreeError::Unconfirmed);
        }
        std::thread::sleep(STOP_POLL);
    }
}

#[cfg(test)]
thread_local! {
    /// Forces the next stop confirmation to fail on this thread.
    static FORCE_UNCONFIRMED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn forced_unconfirmed() -> bool {
    FORCE_UNCONFIRMED.with(std::cell::Cell::get)
}

/// Test-only guard that forces a stop confirmation to fail until it drops.
#[cfg(test)]
pub(crate) struct ForcedUnconfirmed(());

#[cfg(test)]
impl ForcedUnconfirmed {
    pub(crate) fn install() -> Self {
        FORCE_UNCONFIRMED.with(|flag| flag.set(true));
        Self(())
    }
}

#[cfg(test)]
impl Drop for ForcedUnconfirmed {
    fn drop(&mut self) {
        FORCE_UNCONFIRMED.with(|flag| flag.set(false));
    }
}

#[cfg(test)]
mod tests;
