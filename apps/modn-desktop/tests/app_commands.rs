//! Desktop command contracts that span the Rust handlers and the webview API.

use std::fs;
use std::path::Path;

#[expect(
    dead_code,
    reason = "The game-command tests reuse the application backend without invoking its process-global argument parser"
)]
#[cfg(windows)]
#[path = "../src/backend.rs"]
mod backend;

#[expect(
    dead_code,
    reason = "The game-command tests include the command error, busy guard and blocking runner, not every command-layer item"
)]
#[cfg(windows)]
// The path names an existing directory, so rustfmt on Linux can resolve it.
#[path = "../src/app"]
mod app {
    pub(crate) mod blocking;
    pub(crate) mod error;
    pub(crate) mod state;
}

fn read(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"))
}

fn handler_body<'a>(source: &'a str, name: &str) -> &'a str {
    let marker = format!("pub(crate) async fn {name}(");
    let start = source
        .find(&marker)
        .unwrap_or_else(|| panic!("{name} is a command handler"));
    let open = source[start..]
        .find('{')
        .map(|offset| start + offset)
        .unwrap_or_else(|| panic!("{name} has a body"));
    let mut depth = 0usize;
    for (offset, character) in source[open..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[open..open + offset + 1];
                }
            }
            _ => {}
        }
    }
    panic!("{name} body ends");
}

#[test]
fn install_archive_rechecks_the_plan_and_digest_before_the_write() {
    let source = read("src/app/commands/operations.rs");
    let body = handler_body(&source, "install_archive");
    let recheck = body
        .find("ensure_review_unchanged(")
        .expect("install_archive must recheck the review");
    let write = body
        .find("run_write(&app, &state, run, commit)")
        .expect("install_archive must write through run_write");
    assert!(recheck < write, "the recheck must run before the write");
    assert_eq!(
        body.matches("= run_review(").count(),
        2,
        "the review must run again before the write"
    );
}

#[test]
fn install_archive_offers_an_executable_approval() {
    let source = read("src/app/commands/operations.rs");
    let body = handler_body(&source, "install_archive");
    assert!(
        body.contains("executable_approval("),
        "an executable blocker must become an approval ask"
    );
    let approval = body
        .find("CommandError::approval_required(false, candidates)")
        .expect("the ask must carry the exact executable paths");
    let blocked = body
        .find("plan_blocked()")
        .expect("a permanent blocker must stay blocked");
    assert!(
        body.contains("vetted"),
        "the ask must follow a dry run with the candidate approvals"
    );
    let vetted = body
        .find("let plan = archive_dry_run(")
        .expect("the ask must re-run the dry run with the vetted candidates");
    let rechecked = body[vetted..]
        .find("if reviewable(&plan)")
        .map(|offset| vetted + offset)
        .expect("the approved plan must be rechecked before the ask");
    assert!(
        rechecked < approval,
        "only a reviewable vetted plan may ask for approval"
    );
    assert!(
        approval < blocked,
        "only a vetted, reviewable plan may ask for approval"
    );
}

#[test]
fn undo_passes_the_approvals_to_the_cli() {
    let source = read("src/app/commands/history.rs");
    let body = handler_body(&source, "undo");
    assert!(
        body.contains("args::undo(&operation_id, allow_native_injector, &allow_executables)?"),
        "undo must validate the approvals through args::undo"
    );
}

#[test]
fn game_commands_keep_backend_errors_typed() {
    let source = read("src/app/commands/games.rs");
    for name in [
        "detect_games",
        "activate_game",
        "add_game",
        "remove_game",
        "play",
    ] {
        let body = handler_body(&source, name);
        let start = body
            .find("run_blocking(")
            .unwrap_or_else(|| panic!("{name} must run off the UI thread"));
        let job = &body[start..];
        let job = &job[..job
            .find(".await")
            .unwrap_or_else(|| panic!("{name} must await its blocking job"))];
        assert!(
            job.contains("backend.call_with_inactivity_timeout("),
            "{name} must call the inactivity-timeout backend directly"
        );
        assert!(
            !job.contains('?') && !job.contains("Ok("),
            "{name} must return the BackendError from its job instead of flattening it through anyhow"
        );
    }
}

#[cfg(windows)]
mod game_command_errors {
    //! The typed backend error reaches the command error for a game command.

    use super::backend;
    use crate::app;
    use std::ffi::OsString;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    /// A disposable backend state that only these tests may create or remove.
    struct Fixture {
        state: PathBuf,
        backend: backend::Backend,
        game: backend::Game,
    }

    impl Fixture {
        fn new(binary: &str) -> Self {
            static SEQUENCE: AtomicU64 = AtomicU64::new(1);
            let state = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
                "modn-desktop-game-command-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(state.join("game")).expect("create disposable game");
            fs::write(state.join("test-backend.marker"), b"disposable").expect("fixture marker");
            let game = backend::Game {
                game_id: "fixture".into(),
                path: state.join("game"),
            };
            let backend = backend::Backend {
                binary: PathBuf::from(binary),
                state: state.clone(),
                data: None,
                modules: None,
                evidence: None,
                inactivity_timeout: None,
            };
            Self {
                state,
                backend,
                game,
            }
        }

        fn registration_args(&self, scenario: &str) -> Vec<OsString> {
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
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.state).expect("remove only this fixture");
        }
    }

    fn fixture_binary() -> Option<&'static str> {
        // The `verification` feature builds the fixture binary. Without it,
        // Cargo can still name a stale binary from an earlier build, so the
        // feature itself decides whether a fixture exists.
        if !cfg!(feature = "verification") {
            return None;
        }
        option_env!("CARGO_BIN_EXE_modn-desktop-test-backend")
    }

    #[test]
    fn a_game_refusal_keeps_the_cli_code_in_the_command_error() {
        let Some(binary) = fixture_binary() else {
            return;
        };
        let fixture = Fixture::new(binary);
        let busy = Arc::new(app::state::Busy::default());
        let backend = fixture.backend.clone();
        let args = fixture.registration_args("refuse");
        let error = tauri::async_runtime::block_on(app::blocking::run_blocking(&busy, move || {
            backend.call_with_inactivity_timeout(&args)
        }))
        .expect_err("the fixture refuses the registration");
        assert_eq!(error.code, "PLAN_BLOCKED");
        assert!(!error.message.contains("fixture secret"));
    }

    #[test]
    fn a_game_inactivity_timeout_reaches_backend_timeout() {
        let Some(binary) = fixture_binary() else {
            return;
        };
        let fixture = Fixture::new(binary);
        let busy = Arc::new(app::state::Busy::default());
        let backend = fixture
            .backend
            .clone()
            .with_inactivity_timeout(Duration::from_millis(250));
        let args = fixture.registration_args("silent");
        let error = tauri::async_runtime::block_on(app::blocking::run_blocking(&busy, move || {
            backend.call_with_inactivity_timeout(&args)
        }))
        .expect_err("a silent registration must stop");
        assert_eq!(error.code, "BACKEND_TIMEOUT");
    }
}

#[test]
fn the_webview_undo_wrapper_sends_approvals_and_documents_the_error() {
    let source = read("ui/js/api.js");
    let start = source
        .find("export async function undo(")
        .expect("undo wrapper");
    let end = source[start..]
        .find("\n}")
        .map(|offset| start + offset)
        .expect("undo wrapper ends");
    let wrapper = &source[start..end];
    assert!(wrapper.contains("allowNativeInjector"));
    assert!(wrapper.contains("allowExecutables"));
    assert!(
        source.contains("@property {ApprovalRequest} [approval]"),
        "the error's approval field must be documented"
    );
}
