use super::*;
use crate::git::{self, GitEnvironment};
use std::net::TcpListener;
use std::path::Path;

/// The most bytes an error message may add to the kept tail.
const MESSAGE_OVERHEAD: usize = 1024;

fn environment<'a>(directory: &'a Path, hooks: &'a Path) -> GitEnvironment<'a> {
    GitEnvironment {
        directory,
        hooks,
        allow_file: true,
    }
}

/// The test program seam for this thread, removed when the test ends.
struct Program(Option<std::ffi::OsString>);

impl Program {
    fn install(path: &Path) -> Self {
        let previous =
            crate::git::TEST_PROGRAM.with(|slot| slot.replace(Some(path.as_os_str().to_owned())));
        Self(previous)
    }
}

impl Drop for Program {
    fn drop(&mut self) {
        crate::git::TEST_PROGRAM.with(|slot| slot.replace(self.0.take()));
    }
}

/// Write a stand-in Git program that prints the preamble, then `megabytes` of
/// `remote:` messages to its error stream, then one last line, and exits 1.
fn flooding_program(root: &Path, preamble: &[&str], megabytes: usize) -> std::path::PathBuf {
    let flood = root.join("flood.txt");
    let line = "remote: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let mut text = String::with_capacity(1024 * 1024 + line.len());
    while text.len() < 1024 * 1024 {
        text.push_str(line);
        text.push('\n');
    }
    std::fs::write(&flood, &text).expect("flood file");
    let flood = flood.to_string_lossy().replace('\\', "/");
    let directory = root.join("flooding-git");
    std::fs::create_dir_all(&directory).expect("program directory");
    if cfg!(windows) {
        let mut script = String::from("@echo off\r\n");
        for line in preamble {
            script.push_str(&format!("echo {line} 1>&2\r\n"));
        }
        for _ in 0..megabytes {
            script.push_str(&format!("type \"{flood}\" 1>&2\r\n"));
        }
        script.push_str("echo fatal: stub tail 9f3a 1>&2\r\nexit /b 1\r\n");
        let program = directory.join("git.bat");
        std::fs::write(&program, script).expect("program");
        program
    } else {
        let mut script = String::from("#!/bin/sh\n");
        for line in preamble {
            script.push_str(&format!("printf '%s\\n' '{line}' >&2\n"));
        }
        script.push_str("i=0\n");
        script.push_str(&format!(
            "while [ \"$i\" -lt {megabytes} ]; do cat \"{flood}\" >&2; i=$((i+1)); done\n"
        ));
        script.push_str("printf '%s\\n' 'fatal: stub tail 9f3a' >&2\nexit 1\n");
        let program = directory.join("git");
        std::fs::write(&program, script).expect("program");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&program)
                .expect("program metadata")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&program, permissions).expect("program permissions");
        }
        program
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

#[test]
fn a_flood_of_git_error_output_keeps_only_the_tail_and_bounds_the_message() {
    let root = tempfile::tempdir().expect("temp directory");
    let program = flooding_program(root.path(), &[], 16);
    let _program = Program::install(&program);
    let hooks = git::empty_hooks().expect("empty hooks");
    let environment = environment(root.path(), hooks.path());
    let commit = "a".repeat(40);
    let error = fetch_with_command(
        git::command(&environment),
        "https://example.invalid/owner/project.git",
        &commit,
    )
    .expect_err("a stub Git that exits non-zero must fail the fetch");
    let text = format!("{error:#}");
    assert!(!timeout_class(&error), "{text}");
    assert!(text.contains("GIT_SOURCE_FETCH_FAILED"), "{text}");
    assert!(
        text.contains("showing the kept tail"),
        "the message must say the output was truncated: {text}"
    );
    assert!(
        text.contains("fatal: stub tail 9f3a"),
        "the message must hold the last error line; message length {}",
        text.len()
    );
    assert!(
        text.len() <= super::super::run::OUTPUT_TAIL_LIMIT + MESSAGE_OVERHEAD,
        "the message grew past the bound: {} bytes",
        text.len()
    );
}

#[test]
fn a_truncated_low_speed_abort_keeps_the_timeout_class() {
    let root = tempfile::tempdir().expect("temp directory");
    let program = flooding_program(
        root.path(),
        &[
            "fatal: unable to access 'https://example.invalid/': Operation too slow. Less than 1 bytes/sec transferred the last 1 seconds",
        ],
        16,
    );
    let _program = Program::install(&program);
    let hooks = git::empty_hooks().expect("empty hooks");
    let environment = environment(root.path(), hooks.path());
    let commit = "b".repeat(40);
    let error = fetch_with_command(
        git::command(&environment),
        "https://example.invalid/owner/project.git",
        &commit,
    )
    .expect_err("the stub Git exits non-zero after the low-speed line");
    let text = format!("{error:#}");
    assert!(timeout_class(&error), "{text}");
    assert!(
        text.contains("stopped below the Git low-speed limit"),
        "{text}"
    );
    assert!(
        text.contains("showing the kept tail"),
        "the message must say the output was truncated: {text}"
    );
    assert!(
        text.len() <= super::super::run::OUTPUT_TAIL_LIMIT + MESSAGE_OVERHEAD,
        "the message grew past the bound: {} bytes",
        text.len()
    );
}
