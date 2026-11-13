use crate::backend::Backend;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::error::CommandError;

/// Shared application state. The managed backend is absent only when the
/// process started with invalid arguments; a native dialog reports that case.
pub(crate) struct AppState {
    backend: Option<Backend>,
    pub(crate) busy: Arc<Busy>,
}

impl AppState {
    pub(crate) fn new(backend: Option<Backend>) -> Self {
        Self {
            backend,
            busy: Arc::new(Busy::default()),
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

#[cfg(test)]
mod tests {
    use super::Busy;
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
}
