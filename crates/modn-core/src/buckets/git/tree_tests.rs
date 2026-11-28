//! A Git tree that Modn could not confirm stopped ends a catalog sync.
//!
//! The stub Git exits at once, but the stop confirmation is forced to fail.
//! The sync must not try the stored mirror, and it must keep the staging
//! directory that a descendant could still write to.

use crate::acquisition::ForcedUnconfirmed;
use crate::buckets::git::tests as fixtures;
use crate::buckets::BucketStore;
use crate::error::{code_of, ErrorCode};
use modn_core::manager_roots::ManagerRoots;
use std::fs;
use std::path::{Path, PathBuf};

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

/// A stand-in Git that exits with a failure before cloning anything.
fn exiting_program(root: &Path) -> PathBuf {
    let directory = root.join("exiting-git");
    fs::create_dir_all(&directory).expect("program directory");
    if cfg!(windows) {
        let program = directory.join("git.bat");
        fs::write(&program, "@echo off\r\nexit /b 1\r\n").expect("program");
        program
    } else {
        let program = directory.join("git");
        fs::write(&program, "#!/bin/sh\nexit 1\n").expect("program");
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

#[test]
fn an_unconfirmed_tree_stop_ends_the_sync_and_keeps_staging() {
    let key = fixtures::test_key();
    let primary = fixtures::remote("1.0", key);
    let temporary = tempfile::tempdir().expect("temp");
    let state = temporary.path().join("state");
    let store = BucketStore::new(&ManagerRoots::new(&state, None));
    store
        .sync_git("community", Some(fixtures::source(&primary)))
        .expect("the first sync registers the primary");

    // Keep the primary registered and add a mirror that must not be tried.
    let registration = fixtures::registration(&state);
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&registration).expect("registration"))
            .expect("registration JSON");
    value["mirrors"] = serde_json::json!([{
        "kind": "git",
        "remote": "https://example.invalid/mirror.git",
        "revision": "fixture",
    }]);
    fs::write(
        &registration,
        serde_json::to_vec_pretty(&value).expect("registration bytes"),
    )
    .expect("registration write");

    let program = exiting_program(temporary.path());
    let _program = Program::install(&program);
    let _forced = ForcedUnconfirmed::install();
    let error = store
        .sync_git_with_signer("community", None)
        .expect_err("an unconfirmed tree stop must end the sync");
    assert_eq!(
        code_of(&error),
        Some(ErrorCode::GitProcessTreeNotStopped),
        "{error:#}"
    );

    let buckets = state.join("buckets");
    let stages: Vec<PathBuf> = fs::read_dir(&buckets)
        .expect("buckets root")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("git-catalog-"))
        })
        .collect();
    // The first sync kept its staging too, and its successful attempt did
    // produce `catalog`. The failed attempt has an empty `source-0`.
    let failed: Vec<&PathBuf> = stages
        .iter()
        .filter(|stage| stage.join("source-0").is_dir() && !stage.join("source-0/catalog").exists())
        .collect();
    assert_eq!(
        failed.len(),
        1,
        "one failed staging directory must stay: {stages:?}"
    );
    assert!(
        !failed[0].join("source-1").exists(),
        "the mirror must not be tried"
    );
    assert!(
        stages.iter().all(|stage| !stage.join("source-1").exists()),
        "no staging directory may hold a second attempt: {stages:?}"
    );
}
