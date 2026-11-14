//! Shared disposable backend fixture for the desktop backend contract tests.

#[expect(
    dead_code,
    reason = "Reuse the application backend without invoking its process-global argument parser"
)]
#[path = "../../src/backend.rs"]
pub(super) mod backend;

use backend::{Backend, Game};
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub(super) struct Fixture {
    pub(super) backend: Backend,
    pub(super) game: Game,
    pub(super) source: PathBuf,
}

impl Fixture {
    pub(super) fn new() -> Self {
        // Cargo owns this directory and creates it under the target directory,
        // so the caller's TEMP/TMP cannot move fixture state outside the
        // worktree or into the machine-global drive-root temp directory.
        let state = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "modn-desktop-contract-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(state.join("game")).expect("create disposable game");
        fs::write(state.join("test-backend.marker"), b"disposable").expect("fixture marker");
        let source = state.join("source ; $() & spaces.zip");
        fs::write(&source, b"fixture source").expect("fixture archive bytes");
        let game = Game {
            game_id: "fixture".into(),
            path: state.join("game"),
        };
        let backend = Backend {
            binary: PathBuf::from(env!("CARGO_BIN_EXE_modn-desktop-test-backend")),
            state,
            data: None,
            modules: None,
            evidence: None,
            inactivity_timeout: None,
        };
        Self {
            backend,
            game,
            source,
        }
    }
    pub(super) fn mode(&self, mode: &str) {
        fs::write(self.backend.state.join("mode"), mode).expect("mode");
    }
    pub(super) fn no_write(&self) {
        assert!(
            !self.backend.state.join("write.json").exists(),
            "Refusal must not invoke a write"
        );
    }
    pub(super) fn registration_args(&self, scenario: &str) -> Vec<OsString> {
        vec![
            "game".into(),
            "add".into(),
            self.game.game_id.clone().into(),
            "--path".into(),
            self.game.path.clone().into_os_string(),
            "--fixture-scenario".into(),
            scenario.into(),
        ]
    }
    pub(super) fn progress_args(&self, scenario: &str) -> Vec<OsString> {
        let archive = self.backend.state.join(format!("{scenario}.zip"));
        fs::write(&archive, b"fixture source").expect("fixture archive bytes");
        vec![
            "--game".into(),
            self.game.game_id.clone().into(),
            "install".into(),
            archive.into_os_string(),
            "--yes".into(),
            "--path".into(),
            self.game.path.clone().into_os_string(),
        ]
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.backend.state).expect("remove only this fixture");
    }
}
