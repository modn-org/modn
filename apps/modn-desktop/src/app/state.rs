use crate::backend::{Backend, CancelToken};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::error::CommandError;

pub(crate) struct AppState {
    backend: Option<Backend>,
    default_theme: bool,
    themes: Mutex<ThemeAssets>,
    theme_ticket: AtomicU64,
    pub(crate) busy: Arc<Busy>,
    pub(crate) runs: Arc<Runs>,
}

impl AppState {
    pub(crate) fn new(backend: Option<Backend>, default_theme: bool) -> Self {
        Self {
            backend,
            default_theme,
            themes: Mutex::new(ThemeAssets::default()),
            theme_ticket: AtomicU64::new(0),
            busy: Arc::new(Busy::default()),
            runs: Arc::new(Runs::default()),
        }
    }

    pub(crate) fn backend(&self) -> Result<Backend, CommandError> {
        self.backend
            .clone()
            .ok_or_else(CommandError::backend_failed)
    }

    pub(crate) fn default_theme(&self) -> bool {
        self.default_theme
    }

    pub(crate) fn next_theme_ticket(&self) -> u64 {
        let Ok(mut assets) = self.themes.lock() else {
            return self.theme_ticket.fetch_add(1, Ordering::SeqCst) + 1;
        };
        let ticket = self.theme_ticket.fetch_add(1, Ordering::SeqCst) + 1;
        assets.latest_ticket = ticket;
        ticket
    }

    pub(crate) fn store_theme_root_if_latest(&self, ticket: u64, root: Option<PathBuf>) -> bool {
        let Ok(mut assets) = self.themes.lock() else {
            return false;
        };
        if assets.latest_ticket != ticket {
            return false;
        }
        assets.root = root;
        true
    }

    pub(crate) fn theme_root(&self) -> Option<PathBuf> {
        self.themes
            .lock()
            .ok()
            .and_then(|assets| assets.root.clone())
    }
}

#[derive(Default)]
pub(crate) struct ThemeAssets {
    pub(crate) root: Option<PathBuf>,
    latest_ticket: u64,
}

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

#[derive(Default)]
pub(crate) struct Runs {
    tokens: Mutex<HashMap<String, CancelToken>>,
}

impl Runs {
    pub(crate) fn start(self: &Arc<Self>, run_id: &str) -> Result<RunGuard, CommandError> {
        let token = CancelToken::new();
        let mut tokens = self
            .tokens
            .lock()
            .map_err(|_| CommandError::backend_failed())?;
        if tokens.contains_key(run_id) {
            return Err(CommandError::run_in_use());
        }
        tokens.insert(run_id.to_string(), token.clone());
        drop(tokens);
        Ok(RunGuard {
            runs: Arc::clone(self),
            run_id: run_id.to_string(),
            token,
        })
    }

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

    pub(crate) fn run_id(&self) -> &str {
        &self.run_id
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
    use super::{AppState, Busy, Runs};
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
        let run = runs.start("run-1").expect("first start");
        assert!(!run.token().is_cancelled());
        assert!(runs.cancel("run-1"));
        assert!(run.token().is_cancelled());
    }

    #[test]
    fn cancelling_an_unknown_run_is_a_no_op() {
        let runs = Arc::new(Runs::default());
        let run = runs.start("run-1").expect("first start");
        assert!(!runs.cancel("run-2"));
        assert!(!run.token().is_cancelled());
    }

    #[test]
    fn finished_runs_are_removed_from_the_registry() {
        let runs = Arc::new(Runs::default());
        let run = runs.start("run-1").expect("first start");
        drop(run);
        assert!(!runs.cancel("run-1"));
    }

    #[test]
    fn a_second_run_with_the_same_id_is_refused_and_the_first_stays_cancellable() {
        let runs = Arc::new(Runs::default());
        let first = runs.start("run-1").expect("first start");
        let error = runs.start("run-1").err().expect("the id is occupied");
        assert_eq!(error.code, "RUN_ID_IN_USE");
        assert!(runs.cancel("run-1"));
        assert!(first.token().is_cancelled());
    }

    #[test]
    fn the_first_guard_removes_only_its_own_entry() {
        let runs = Arc::new(Runs::default());
        let first = runs.start("run-1").expect("first start");
        drop(first);

        let second = runs.start("run-1").expect("the id is free again");
        assert!(runs.cancel("run-1"));
        assert!(second.token().is_cancelled());
        drop(second);
        assert!(!runs.cancel("run-1"));
    }

    #[test]
    fn the_latest_ticket_stores_replaces_and_clears_the_theme_root() {
        let state = AppState::new(None, false);
        assert!(state.theme_root().is_none());
        let first = std::env::temp_dir().join("modn-desktop-themed-first");
        let second = std::env::temp_dir().join("modn-desktop-themed-second");
        let ticket = state.next_theme_ticket();
        assert!(state.store_theme_root_if_latest(ticket, Some(first.clone())));
        assert_eq!(state.theme_root(), Some(first));
        let ticket = state.next_theme_ticket();
        assert!(state.store_theme_root_if_latest(ticket, Some(second.clone())));
        assert_eq!(state.theme_root(), Some(second));
        let ticket = state.next_theme_ticket();
        assert!(state.store_theme_root_if_latest(ticket, None));
        assert!(state.theme_root().is_none());
    }

    #[test]
    fn an_older_ticket_finishing_last_leaves_the_newer_root() {
        let state = AppState::new(None, false);
        let older = state.next_theme_ticket();
        let newer = state.next_theme_ticket();
        let older_root = std::env::temp_dir().join("modn-desktop-themed-older");
        let newer_root = std::env::temp_dir().join("modn-desktop-themed-newer");
        assert!(state.store_theme_root_if_latest(newer, Some(newer_root.clone())));
        assert!(!state.store_theme_root_if_latest(older, Some(older_root)));
        assert_eq!(state.theme_root(), Some(newer_root));
    }

    #[test]
    fn a_superseded_failure_does_not_clear_the_newer_root() {
        let state = AppState::new(None, false);
        let older = state.next_theme_ticket();
        let newer = state.next_theme_ticket();
        let newer_root = std::env::temp_dir().join("modn-desktop-themed-newer");
        assert!(state.store_theme_root_if_latest(newer, Some(newer_root.clone())));
        assert!(!state.store_theme_root_if_latest(older, None));
        assert_eq!(state.theme_root(), Some(newer_root));
    }

    #[test]
    fn the_default_theme_flag_is_read_from_the_state() {
        assert!(AppState::new(None, true).default_theme());
        assert!(!AppState::new(None, false).default_theme());
    }
}
