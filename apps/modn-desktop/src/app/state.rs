use crate::backend::{Backend, CancelToken};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::error::CommandError;

/// Shared application state. The managed backend is absent only when the
/// process started with invalid arguments; a native dialog reports that case.
pub(crate) struct AppState {
    backend: Option<Backend>,
    pub(crate) busy: Arc<Busy>,
    pub(crate) runs: Arc<Runs>,
}

impl AppState {
    pub(crate) fn new(backend: Option<Backend>) -> Self {
        Self {
            backend,
            busy: Arc::new(Busy::default()),
            runs: Arc::new(Runs::default()),
        }
    }

    pub(crate) fn backend(&self) -> Result<Backend, CommandError> {
        self.backend
            .clone()
            .ok_or_else(CommandError::backend_failed)
    }
}

/// Counts backend calls. The close handler reads it synchronously to veto a
/// close while a write may still be running.
#[derive(Default)]
pub(crate) struct Busy {
    count: AtomicUsize,
}

impl Busy {
    pub(crate) fn begin(self: &Arc<Self>) -> BusyGuard {
        self.count.fetch_add(1, Ordering::SeqCst);
        BusyGuard {
            busy: Arc::clone(self),
        }
    }

    pub(crate) fn is_busy(&self) -> bool {
        self.count.load(Ordering::SeqCst) > 0
    }
}

pub(crate) struct BusyGuard {
    busy: Arc<Busy>,
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.busy.count.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Maps the webview's run id to the cancel token of the running write.
///
/// The webview creates one id per operation and passes it to the write command
/// and to `cancel_run`. A completed or failed run removes its own entry, so a
/// late cancel for an old id is a no-op.
#[derive(Default)]
pub(crate) struct Runs {
    tokens: Mutex<HashMap<String, CancelToken>>,
}

impl Runs {
    /// Registers one run. The returned guard removes the entry when dropped.
    pub(crate) fn start(self: &Arc<Self>, run_id: &str) -> RunGuard {
        let token = CancelToken::new();
        if let Ok(mut tokens) = self.tokens.lock() {
            tokens.insert(run_id.to_string(), token.clone());
        }
        RunGuard {
            runs: Arc::clone(self),
            run_id: run_id.to_string(),
            token,
        }
    }

    /// Cancels a running write and reports whether the id was known.
    pub(crate) fn cancel(&self, run_id: &str) -> bool {
        let Ok(tokens) = self.tokens.lock() else {
            return false;
        };
        match tokens.get(run_id) {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }
}

pub(crate) struct RunGuard {
    runs: Arc<Runs>,
    run_id: String,
    token: CancelToken,
}

impl RunGuard {
    pub(crate) fn token(&self) -> CancelToken {
        self.token.clone()
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        if let Ok(mut tokens) = self.runs.tokens.lock() {
            tokens.remove(&self.run_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Busy, Runs};
    use std::sync::Arc;

    #[test]
    fn busy_guard_tracks_running_calls() {
        let busy = Arc::new(Busy::default());
        assert!(!busy.is_busy());
        let guard = busy.begin();
        assert!(busy.is_busy());
        drop(guard);
        assert!(!busy.is_busy());
    }

    #[test]
    fn cancel_reaches_the_token_of_a_running_run() {
        let runs = Arc::new(Runs::default());
        let run = runs.start("run-1");
        assert!(!run.token().is_cancelled());
        assert!(runs.cancel("run-1"));
        assert!(run.token().is_cancelled());
    }

    #[test]
    fn cancelling_an_unknown_run_is_a_no_op() {
        let runs = Arc::new(Runs::default());
        let run = runs.start("run-1");
        assert!(!runs.cancel("run-2"));
        assert!(!run.token().is_cancelled());
    }

    #[test]
    fn finished_runs_are_removed_from_the_registry() {
        let runs = Arc::new(Runs::default());
        let run = runs.start("run-1");
        drop(run);
        assert!(!runs.cancel("run-1"));
    }
}
