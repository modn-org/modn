//! The staging-directory branch of [`GitSource::acquire_url`].
//!
//! When the Git process tree cannot be confirmed stopped, a descendant may
//! still write into the staging directory. The acquisition keeps that
//! directory instead of letting its owner remove it under the writer, and it
//! stores no cache entry.

use super::tree::ForcedUnconfirmed;
use super::{ArchiveCache, CacheUse, GitCacheKey, GitSource};
use crate::error::{code_of, ErrorCode};
use crate::progress::OperationControl;
use std::fs;
use std::path::{Path, PathBuf};

/// The remote that the stub Git fails without forwarding.
const REMOTE: &str = "https://example.invalid/fixture/project.git";

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

fn real_git() -> PathBuf {
    let name = if cfg!(windows) { "git.exe" } else { "git" };
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
        .expect("the test needs Git on PATH")
}

/// Write a stand-in Git that exits 1 for the fetch of [`REMOTE`] and forwards
/// every other command to the real Git.
///
/// The stub exits on its own, so the test leaves no process behind.
fn exiting_fetch_program(root: &Path) -> PathBuf {
    let directory = root.join("exiting-git");
    fs::create_dir_all(&directory).expect("program directory");
    let real = real_git().to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        let program = directory.join("git.bat");
        let script = format!(
            "@echo off\r\necho %* | findstr /C:\"{REMOTE}\" >nul\r\nif errorlevel 1 goto real\r\nexit /b 1\r\n:real\r\n\"{real}\" %*\r\n"
        );
        fs::write(&program, script).expect("program");
        program
    } else {
        let program = directory.join("git");
        let script = format!(
            "#!/bin/sh\ncase \"$*\" in\n  *{REMOTE}*) exit 1 ;;\n  *) exec \"{real}\" \"$@\" ;;\nesac\n"
        );
        fs::write(&program, script).expect("program");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&program)
                .expect("program metadata")
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&program, permissions).expect("program permissions");
        }
        program
    }
}

/// Every `git-source-*` directory under the cache root.
fn staging_directories(root: &Path) -> Vec<PathBuf> {
    fs::read_dir(root)
        .expect("cache root")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("git-source-"))
        })
        .collect()
}

/// Every archive file under the cache root, at any depth.
fn archives(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).expect("cache directory") {
            let path = entry.expect("cache entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "zip") {
                found.push(path);
            }
        }
    }
    found
}

#[test]
fn an_unconfirmed_tree_stop_keeps_the_staging_directory() {
    let root = tempfile::tempdir().expect("temp directory");
    let program = exiting_fetch_program(root.path());
    let _program = Program::install(&program);
    let cache = ArchiveCache::new(root.path().join("cache"));
    let commit = "a".repeat(40);
    let key = GitCacheKey::new("test-game", "fixture", "1.0", &commit);
    let source = GitSource::new("fixture/project".into(), commit.clone());
    let _forced = ForcedUnconfirmed::install();

    let error = source
        .acquire_url(
            REMOTE,
            &cache,
            &key,
            CacheUse::Refresh,
            &OperationControl::default(),
        )
        .expect_err("a forced unconfirmed tree stop must fail the acquisition");
    let text = format!("{error:#}");
    assert_eq!(
        code_of(&error),
        Some(ErrorCode::GitProcessTreeNotStopped),
        "{text}"
    );
    assert!(text.contains("GIT_PROCESS_TREE_NOT_STOPPED"), "{text}");

    let kept = staging_directories(cache.root());
    assert_eq!(
        kept.len(),
        1,
        "the kept staging directory must stay in the cache root: {kept:?}"
    );
    assert!(kept[0].is_dir(), "{}", kept[0].display());
    assert!(
        kept[0].join("source.git").is_dir(),
        "the fetch must have reached the staging directory: {}",
        kept[0].display()
    );
    let archives = archives(cache.root());
    assert!(
        archives.is_empty(),
        "the failed acquisition must store no archive: {archives:?}"
    );
}
