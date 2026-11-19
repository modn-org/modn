//! The Git fetch process trees that this process started and has not ended.

use std::collections::BTreeSet;
use std::sync::{Mutex, MutexGuard, PoisonError};

/// The process id of one fetch child.
///
/// On Unix the child leads its own process group, so the same number names the
/// group that ends the whole tree. Windows records the child id only.
type FetchKey = u32;

static LIVE: Mutex<BTreeSet<FetchKey>> = Mutex::new(BTreeSet::new());

#[expect(
    clippy::disallowed_methods,
    reason = "the registry lock holds only set operations and cannot panic; a poisoned lock must still yield its trees to an interrupt handler"
)]
fn live() -> MutexGuard<'static, BTreeSet<FetchKey>> {
    LIVE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Tests that end every live tree take this gate first, so no new tree
/// registers between their last check and their kill.
#[cfg(test)]
static QUIESCE: Mutex<()> = Mutex::new(());

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "a failed test poisons the gate, and a later test should still run instead of panicking"
)]
fn gate() -> MutexGuard<'static, ()> {
    QUIESCE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Stop new fetch trees from registering until the guard drops.
#[cfg(all(test, unix))]
pub(super) fn quiesce() -> MutexGuard<'static, ()> {
    gate()
}

/// Keep one started fetch tree registered until this guard drops.
pub(super) struct Registration {
    key: FetchKey,
    keep: bool,
}

impl Registration {
    /// Leave the tree registered so an interrupt handler can try again.
    ///
    /// The timeout path calls this when it cannot stop the tree. The entry
    /// stays until the process exits; a later kill of the group is harmless
    /// when the group already ended.
    pub(super) fn keep(&mut self) {
        self.keep = true;
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        if !self.keep {
            live().remove(&self.key);
        }
    }
}

/// Register a fetch tree that just started.
///
/// A signal that arrives after the spawn but before this call is not covered:
/// the tree is not in the registry yet.
pub(super) fn register(key: FetchKey) -> Registration {
    #[cfg(test)]
    let _quiet = gate();
    live().insert(key);
    Registration { key, keep: false }
}

/// End every registered fetch tree.
pub(crate) fn end_active_fetch_trees() {
    #[cfg(unix)]
    {
        use rustix::process::{kill_process_group, Pid, Signal};
        let trees: Vec<FetchKey> = live().iter().copied().collect();
        for tree in trees {
            let Ok(raw) = i32::try_from(tree) else {
                continue;
            };
            let Some(group) = Pid::from_raw(raw) else {
                continue;
            };
            // The group may already be gone, and a process id can be reused;
            // every error is ignored for the same reason.
            let _ = kill_process_group(group, Signal::KILL);
        }
    }
    #[cfg(not(unix))]
    {
        // Windows has no process group to signal; the bookkeeping still runs.
    }
}

/// The number of live fetch trees, for tests that observe the registry.
#[cfg(test)]
pub(super) fn active_count() -> usize {
    live().len()
}

/// Whether this fetch tree is registered, for tests that watch one tree.
#[cfg(all(test, unix))]
pub(super) fn contains(key: FetchKey) -> bool {
    live().contains(&key)
}
