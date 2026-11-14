#![cfg(windows)]

#[path = "backend_contract/support.rs"]
mod support;

use std::fs;
use std::time::{Duration, Instant};
use support::Fixture;
use support::backend::{BackendErrorKind, CancelToken};

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
    assert!(
        error.to_string().contains("60 seconds"),
        "unexpected backend error: {error}"
    );
    assert!(
        start.elapsed().as_secs() < 65,
        "Deadline must remain bounded"
    );
    f.no_write();
}

#[test]
fn progress_events_arrive_in_order_and_return_the_result() {
    let f = Fixture::new();
    let args = f.progress_args("complete");
    let cancel = CancelToken::new();
    let mut events = Vec::new();
    let result = f
        .backend
        .call_with_progress(&args, &cancel, &mut |event| events.push(event))
        .expect("progress install");
    assert_eq!(result["id"], "owned-package");
    let phases: Vec<&str> = events.iter().map(|event| event.phase.as_str()).collect();
    assert_eq!(
        phases,
        [
            "resolve", "download", "extract", "stage", "commit", "result"
        ]
    );
    assert_eq!(events[1].bytes, Some(512));
    assert_eq!(events[1].total, Some(1024));
    assert_eq!(
        events.last().expect("result event").outcome.as_deref(),
        Some("succeeded")
    );
    assert!(f.backend.state.join("write.json").is_file());
    let recorded: Vec<String> = serde_json::from_slice(
        &fs::read(f.backend.state.join("last-args.json")).expect("recorded args"),
    )
    .expect("argv JSON");
    assert!(
        recorded
            .windows(2)
            .any(|pair| pair == ["--progress", "json"])
    );
}

#[test]
fn cancel_before_commit_returns_cancelled_and_writes_nothing() {
    let f = Fixture::new();
    let args = f.progress_args("hang-before-commit");
    let cancel = CancelToken::new();
    let error = f
        .backend
        .call_with_progress(&args, &cancel, &mut |event| {
            if event.phase == "stage" {
                cancel.cancel();
            }
        })
        .expect_err("cancel must stop before the commit");
    assert_eq!(error.kind(), BackendErrorKind::Cancelled);
    assert_eq!(error.code(), Some("CANCELLED"));
    f.no_write();
}

#[test]
fn a_stalled_cancel_keeps_the_pre_commit_inactivity_deadline() {
    let f = Fixture::new();
    let args = f.progress_args("stall-after-cancel");
    // Long enough that a loaded machine still delivers `stage` before it.
    let backend = f
        .backend
        .clone()
        .with_inactivity_timeout(Duration::from_secs(2));
    let cancel = CancelToken::new();
    let start = Instant::now();
    let error = backend
        .call_with_progress(&args, &cancel, &mut |event| {
            if event.phase == "stage" {
                cancel.cancel();
            }
        })
        .expect_err("a step that stalls after a cancel must not wait forever");
    assert_eq!(error.kind(), BackendErrorKind::Cancelled);
    assert_eq!(error.code(), Some("CANCELLED"));
    assert!(
        start.elapsed() < Duration::from_secs(25),
        "the pre-commit inactivity deadline must still end a stalled cancel"
    );
    f.no_write();
}

#[test]
fn an_oversized_result_after_commit_reports_a_committed_outcome() {
    let f = Fixture::new();
    let args = f.progress_args("oversized-after-commit");
    let error = f
        .backend
        .call_with_progress(&args, &CancelToken::new(), &mut |_| {})
        .expect_err("an unreadable result must still report the commit");
    assert_eq!(error.kind(), BackendErrorKind::Committed);
    assert_eq!(error.code(), Some("BACKEND_OUTPUT_TRUNCATED_AFTER_COMMIT"));
    assert!(f.backend.state.join("write.json").is_file());
    assert!(
        f.backend.state.join("survived-after-output").is_file(),
        "a committed writer must never be killed for output size"
    );
}

#[test]
fn a_failed_exit_after_commit_outranks_lost_output() {
    let f = Fixture::new();
    let args = f.progress_args("overflow-failure-after-commit");
    let error = f
        .backend
        .call_with_progress(&args, &CancelToken::new(), &mut |_| {})
        .expect_err("a failed commit must not report a completed outcome");
    assert_eq!(error.kind(), BackendErrorKind::Refused);
    assert_eq!(error.code(), Some("INSTALL_FAILED"));
    assert_ne!(
        error.code(),
        Some("BACKEND_OUTPUT_TRUNCATED_AFTER_COMMIT"),
        "the exit status decides whether the commit completed"
    );
    assert!(f.backend.state.join("write.json").is_file());
}

#[test]
fn an_approval_refusal_keeps_the_native_flag_and_exact_executables() {
    let f = Fixture::new();
    let args = f.progress_args("approval-both");
    let error = f
        .backend
        .call_with_progress(&args, &CancelToken::new(), &mut |_| {})
        .expect_err("the approval refusal must fail");
    assert_eq!(error.kind(), BackendErrorKind::Refused);
    assert_eq!(error.code(), Some("NATIVE_INJECTOR_NOT_ALLOWED"));
    let approval = error
        .approval()
        .expect("the refusal names its uncovered capabilities");
    assert!(approval.native_injector);
    assert_eq!(
        approval.executables,
        ["tools/native-helper.exe", "engine/tools/inject.exe"]
    );
    assert!(approval.complete);
    f.no_write();
}

#[test]
fn cancel_after_commit_lets_the_operation_finish() {
    let f = Fixture::new();
    let args = f.progress_args("hang-after-commit");
    let cancel = CancelToken::new();
    let result = f
        .backend
        .call_with_progress(&args, &cancel, &mut |event| {
            if event.phase == "commit" {
                cancel.cancel();
            }
        })
        .expect("cancel after the commit is ignored");
    assert_eq!(result["id"], "owned-package");
    assert!(
        f.backend.state.join("cancel-after-commit").is_file(),
        "The call must still write cancel after the commit"
    );
    assert!(f.backend.state.join("write.json").is_file());
}

#[test]
fn the_child_environment_is_cleared_except_for_the_allowlist() {
    let f = Fixture::new();
    let mut args = f.progress_args("complete");
    args.push("--record-env".into());
    f.backend
        .call_with_progress(&args, &CancelToken::new(), &mut |_| {})
        .expect("environment probe");
    let environment: serde_json::Value = serde_json::from_slice(
        &fs::read(f.backend.state.join("environment.json")).expect("environment record"),
    )
    .expect("environment JSON");
    assert_eq!(environment["SystemRoot"], true);
    for name in ["TEMP", "TMP"] {
        let absolute =
            std::env::var_os(name).is_some_and(|value| std::path::Path::new(&value).is_absolute());
        assert_eq!(
            environment[name], absolute,
            "{name} is forwarded only as an absolute Windows path"
        );
    }
    for name in [
        "LOCALAPPDATA",
        "MODN_CACHE",
        "MODN_STORE_LIBRARIES",
        "MODN_APPDATA",
        "MODN_GAME_MODULES_DIR",
    ] {
        let present = std::env::var_os(name).is_some();
        assert_eq!(
            environment[name], present,
            "{name} is part of the fixed allowlist"
        );
    }
    assert_eq!(
        environment["PATH"], false,
        "The child environment must be cleared except for the allowlist"
    );
}

#[test]
fn inactivity_before_commit_cancels_then_returns_a_timeout() {
    let f = Fixture::new();
    let args = f.progress_args("hang-before-commit");
    let backend = f
        .backend
        .clone()
        .with_inactivity_timeout(Duration::from_millis(200));
    let start = Instant::now();
    let error = backend
        .call_with_progress(&args, &CancelToken::new(), &mut |_| {})
        .expect_err("inactivity must stop the call");
    assert_eq!(error.kind(), BackendErrorKind::Timeout);
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "A CLI that stops on cancel must not use the whole grace period"
    );
    f.no_write();
}

#[test]
fn inactivity_after_commit_waits_for_the_cli() {
    let f = Fixture::new();
    let args = f.progress_args("hang-after-commit");
    let backend = f
        .backend
        .clone()
        .with_inactivity_timeout(Duration::from_millis(200));
    let start = Instant::now();
    let result = backend
        .call_with_progress(&args, &CancelToken::new(), &mut |_| {})
        .expect("a committed operation must finish");
    assert_eq!(result["id"], "owned-package");
    assert!(
        start.elapsed() >= Duration::from_millis(1400),
        "The fixture finishes only after its post-commit delay"
    );
    assert!(f.backend.state.join("write.json").is_file());
}

#[test]
fn progress_errors_expose_the_code_but_never_stderr_text() {
    let f = Fixture::new();
    let args = f.progress_args("refuse");
    let error = f
        .backend
        .call_with_progress(&args, &CancelToken::new(), &mut |_| {})
        .expect_err("refusal must fail");
    assert_eq!(error.kind(), BackendErrorKind::Refused);
    assert_eq!(error.code(), Some("PLAN_BLOCKED"));
    let message = error.to_string();
    assert!(!message.contains("fixture stderr noise"));
    assert!(!message.contains("fixture secret"));
    f.no_write();
}

#[test]
fn progress_activity_resets_the_inactivity_timer() {
    let f = Fixture::new();
    let args = f.progress_args("slow");
    let backend = f
        .backend
        .clone()
        .with_inactivity_timeout(Duration::from_millis(800));
    let result = backend
        .call_with_progress(&args, &CancelToken::new(), &mut |_| {})
        .expect("each event must reset the inactivity timer");
    assert_eq!(result["id"], "owned-package");
    assert!(f.backend.state.join("write.json").is_file());
}

#[test]
fn game_registration_uses_the_inactivity_timeout() {
    let f = Fixture::new();
    let args = f.registration_args("silent");
    let backend = f
        .backend
        .clone()
        .with_inactivity_timeout(Duration::from_secs(1));
    let start = Instant::now();
    let error = backend
        .call_with_inactivity_timeout(&args)
        .expect_err("a silent registration must stop");
    assert_eq!(error.kind(), BackendErrorKind::Timeout);
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "the configured inactivity timeout must replace the fixed deadline"
    );
    let recorded: Vec<String> = serde_json::from_slice(
        &fs::read(f.backend.state.join("last-args.json")).expect("recorded args"),
    )
    .expect("argv JSON");
    assert!(
        !recorded.iter().any(|arg| arg == "--progress"),
        "game registration does not report progress events"
    );
}

#[test]
fn game_registration_survives_while_output_stays_active() {
    let f = Fixture::new();
    let args = f.registration_args("heartbeat");
    let backend = f
        .backend
        .clone()
        .with_inactivity_timeout(Duration::from_secs(2));
    let result = backend
        .call_with_inactivity_timeout(&args)
        .expect("each output line must reset the inactivity timer");
    assert_eq!(result["game_id"], "fixture");
}

#[test]
fn game_registration_is_not_killed_after_stdout_reports_the_result() {
    let f = Fixture::new();
    let args = f.registration_args("committed-stall");
    let backend = f
        .backend
        .clone()
        .with_inactivity_timeout(Duration::from_secs(2));
    let result = backend
        .call_with_inactivity_timeout(&args)
        .expect("a result on stdout means the write committed");
    assert_eq!(result["game_id"], "fixture");
    assert!(f.backend.state.join("committed").is_file());
    assert!(
        f.backend.state.join("survived-after-result").is_file(),
        "the backend process must never be killed after printing its result"
    );
}

#[test]
fn game_launch_returns_before_the_started_game_exits() {
    let f = Fixture::new();
    let args = f.registration_args("game-launch");
    let backend = f
        .backend
        .clone()
        .with_inactivity_timeout(Duration::from_secs(5));
    let start = Instant::now();
    let result = backend
        .call_with_inactivity_timeout(&args)
        .expect("launch result");
    let elapsed = start.elapsed();
    assert_eq!(result["game_id"], "fixture");
    assert!(
        elapsed < Duration::from_secs(3),
        "the desktop must not wait for the started game: {elapsed:?}"
    );
}
