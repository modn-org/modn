#![cfg(windows)]

#[expect(
    dead_code,
    reason = "Reuse the application backend without invoking its process-global argument parser"
)]
#[path = "../src/backend.rs"]
mod backend;

use backend::{Backend, Game};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct Fixture {
    backend: Backend,
    game: Game,
    source: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let state = std::env::temp_dir().join(format!(
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
        };
        Self {
            backend,
            game,
            source,
        }
    }
    fn mode(&self, mode: &str) {
        fs::write(self.backend.state.join("mode"), mode).expect("mode");
    }
    fn no_write(&self) {
        assert!(
            !self.backend.state.join("write.json").exists(),
            "Refusal must not invoke a write"
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.backend.state).expect("remove only this fixture");
    }
}

#[test]
fn review_is_read_only_and_install_passes_a_literal_source_argument() {
    let f = Fixture::new();
    let review = f.backend.review(&f.game, &f.source).expect("review");
    f.no_write();
    f.backend.install(&review).expect("explicit install");
    let args: Vec<String> = serde_json::from_slice(
        &fs::read(f.backend.state.join("write.json")).expect("write record"),
    )
    .expect("argv JSON");
    assert!(
        args.iter()
            .any(|arg| std::path::Path::new(arg) == review.archive)
    );
    assert!(args.iter().any(|arg| arg == "--yes"));
    assert!(!args.iter().any(|arg| arg == "--allow-executable"));
}

#[test]
fn changed_source_registration_and_plan_refuse_before_a_write() {
    for mode in ["source-changed", "registration-changed", "blocked"] {
        let f = Fixture::new();
        let review = f.backend.review(&f.game, &f.source).expect("review");
        if mode == "source-changed" {
            fs::write(&f.source, b"changed").expect("modify source");
        } else {
            f.mode(mode);
        }
        assert!(f.backend.install(&review).is_err(), "Must refuse {mode}");
        f.no_write();
    }
}

#[test]
fn strict_uninstall_requires_a_current_owned_package() {
    let f = Fixture::new();
    assert!(f.backend.uninstall(&f.game, "not-owned").is_err());
    f.no_write();
    f.backend
        .uninstall(&f.game, "owned-package")
        .expect("strict uninstall");
    let args = fs::read_to_string(f.backend.state.join("write.json")).expect("write record");
    assert!(args.contains("uninstall"));
    assert!(!args.contains("force"));
}

#[test]
fn bounded_output_failure_and_invalid_json_are_visible_errors() {
    for mode in ["overflow", "failure", "invalid-json"] {
        let f = Fixture::new();
        f.mode(mode);
        let error = f.backend.games().expect_err("Backend must fail closed");
        assert!(!error.to_string().is_empty());
        f.no_write();
    }
}

#[test]
fn timeout_reaps_the_backend_process() {
    let f = Fixture::new();
    f.mode("timeout");
    let start = std::time::Instant::now();
    let error = f.backend.games().expect_err("deadline");
    assert!(error.to_string().contains("60 seconds"));
    assert!(
        start.elapsed().as_secs() < 65,
        "Deadline must remain bounded"
    );
    f.no_write();
}
