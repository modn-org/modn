use super::*;
use crate::git::{self, GitEnvironment};
use std::net::TcpListener;
use std::path::Path;

fn environment<'a>(directory: &'a Path, hooks: &'a Path) -> GitEnvironment<'a> {
    GitEnvironment {
        directory,
        hooks,
        allow_file: true,
    }
}

fn timeout_class(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<FetchError>(),
            Some(FetchError::Timeout)
        )
    })
}

#[test]
fn the_shared_git_command_passes_the_low_speed_limits() {
    let hooks = git::empty_hooks().expect("empty hooks");
    let command = git::command(&environment(hooks.path(), hooks.path()));
    let arguments: Vec<String> = command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair == ["-c", "http.lowSpeedLimit=1"]),
        "the shared command omits the low-speed limit: {arguments:?}"
    );
    let expected = format!("http.lowSpeedTime={}", git::low_speed_time_seconds());
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair[0] == "-c" && pair[1] == expected),
        "the shared command omits {expected}: {arguments:?}"
    );
    assert_eq!(
        git::default_low_speed_time_seconds(),
        90,
        "the low-speed time must match the documented inactivity limit"
    );
}

#[test]
fn a_real_low_speed_abort_keeps_the_timeout_class() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("stalling TCP listener");
    let port = listener.local_addr().expect("listener address").port();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for connection in listener.incoming() {
            match connection {
                Ok(connection) => held.push(connection),
                Err(_) => break,
            }
        }
    });

    let root = tempfile::tempdir().expect("temp directory");
    let hooks = git::empty_hooks().expect("empty hooks");
    let environment = environment(root.path(), hooks.path());
    git::run(&environment, &["init", "--bare", "--quiet"]).expect("bare repository");

    let url = format!("http://127.0.0.1:{port}/repo.git");
    let commit = "0".repeat(40);
    let command = git::command_with_low_speed_time(&environment, 1);
    let error =
        fetch_with_command(command, &url, &commit).expect_err("a stalled HTTP transfer must fail");
    let text = format!("{error:#}");
    assert!(timeout_class(&error), "{text}");
    assert!(
        text.contains("Operation too slow"),
        "the test must use Git's real low-speed text: {text}"
    );
}

#[test]
fn a_fetch_failure_without_low_speed_text_is_not_a_timeout() {
    let root = tempfile::tempdir().expect("temp directory");
    let hooks = git::empty_hooks().expect("empty hooks");
    let environment = environment(root.path(), hooks.path());
    git::run(&environment, &["init", "--bare", "--quiet"]).expect("bare repository");
    let missing = root.path().join("missing.git");
    let missing = missing.to_str().expect("missing repository path");
    let error = fetch_with(&environment, missing, &"0".repeat(40))
        .expect_err("a missing repository must fail");
    let text = format!("{error:#}");
    assert!(!timeout_class(&error), "{text}");
    assert!(text.contains("failed"), "{text}");
}
