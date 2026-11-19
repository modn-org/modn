//! A stalled catalog mirror falls over under the bounded fetch, or stops it.

use super::CatalogHost;
use crate::buckets::git::tests as fixtures;
use crate::buckets::git::{CatalogFetch, GitCatalogSource};
use crate::buckets::BucketStore;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Write a stand-in Git program that stalls its first clone for a minute.
///
/// Every call appends a line to `log`. The first call writes `flag` and sleeps
/// without output, which the bounded catalog fetch must end after its
/// inactivity limit. Later calls run the real Git with the same arguments, so a
/// next mirror could serve the catalog.
fn stalling_program(root: &Path, flag: &Path, log: &Path) -> PathBuf {
    let flag = flag.to_string_lossy().replace('\\', "/");
    let log = log.to_string_lossy().replace('\\', "/");
    let directory = root.join("stalling-git");
    fs::create_dir_all(&directory).expect("program directory");
    if cfg!(windows) {
        let program = directory.join("git.bat");
        fs::write(
            &program,
            format!(
                "@echo off\r\necho call>>\"{log}\"\r\nif exist \"{flag}\" goto real\r\necho stalled > \"{flag}\"\r\nping -n 60 127.0.0.1 >nul\r\nexit /b 1\r\n:real\r\ngit %*\r\n"
            ),
        )
        .expect("program");
        program
    } else {
        let program = directory.join("git");
        fs::write(
            &program,
            format!(
                "#!/bin/sh\necho call >> \"{log}\"\nif [ ! -f \"{flag}\" ]; then : > \"{flag}\"; sleep 60; exit 1; fi\nexec git \"$@\"\n"
            ),
        )
        .expect("program");
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

/// A registered catalog whose primary stalls and whose stored mirror can serve.
struct Registered {
    temp: tempfile::TempDir,
    store: BucketStore,
    mirror: GitCatalogSource,
    _primary: fixtures::Remote,
    _mirror: fixtures::Remote,
}

/// Register a catalog from the primary and store the mirror after it.
fn registered() -> Registered {
    let key = fixtures::test_key();
    let primary = fixtures::remote("1.0", key.clone());
    let mirror = fixtures::remote("1.0", key);
    let temp = tempfile::tempdir().expect("temp");
    let state = temp.path().join("state");
    let store = BucketStore::new(&state);
    store
        .sync_git("community", Some(fixtures::source(&primary)))
        .expect("the first sync registers the primary");

    // The primary stays the registered source. A stalled first attempt must
    // therefore reach the stored mirror that follows it.
    let mirror_source = fixtures::source(&mirror);
    let registration = fixtures::registration(&state);
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&registration).expect("registration"))
            .expect("registration JSON");
    value["mirrors"] = serde_json::json!([{
        "kind": "git",
        "remote": mirror_source.remote,
        "revision": mirror_source.reference,
    }]);
    fs::write(
        &registration,
        serde_json::to_vec_pretty(&value).expect("registration bytes"),
    )
    .expect("registration write");
    Registered {
        temp,
        store,
        mirror: mirror_source,
        _primary: primary,
        _mirror: mirror,
    }
}

#[test]
fn a_stalled_first_mirror_times_out_and_the_next_mirror_serves() {
    let fixture = registered();
    let flag = fixture.temp.path().join("stalled");
    let log = fixture.temp.path().join("calls.log");
    let program = stalling_program(fixture.temp.path(), &flag, &log);
    let started = Instant::now();
    let synced = fixture
        .store
        .sync_git_with_fetch(
            "community",
            None,
            &CatalogFetch {
                limit: Duration::from_millis(3000),
                program: Some(program),
                fail_tree_stop: false,
            },
        )
        .expect("the second mirror serves the catalog");
    assert!(
        started.elapsed() >= Duration::from_millis(3000),
        "the stalled mirror failed before the inactivity limit: {:?}",
        started.elapsed()
    );
    assert_eq!(
        synced.trace.served_by,
        CatalogHost::from(&fixture.mirror),
        "the mirror served the catalog"
    );
    assert!(
        synced.trace.failed_sources.len() == 1,
        "one mirror failed: {:?}",
        synced.trace.failed_sources
    );
    assert!(
        synced.trace.failed_sources[0].contains("timed out"),
        "the stalled mirror is recorded as a timeout: {:?}",
        synced.trace.failed_sources
    );
}

#[test]
fn an_unstoppable_mirror_tree_stops_the_sync_without_trying_the_next_mirror() {
    let fixture = registered();
    let flag = fixture.temp.path().join("stalled");
    let log = fixture.temp.path().join("calls.log");
    let program = stalling_program(fixture.temp.path(), &flag, &log);
    let error = fixture
        .store
        .sync_git_with_fetch(
            "community",
            None,
            &CatalogFetch {
                limit: Duration::from_millis(800),
                program: Some(program),
                fail_tree_stop: true,
            },
        )
        .expect_err("a tree that cannot be stopped must stop the sync");
    assert!(
        format!("{error:#}").contains("process tree could not be stopped"),
        "{error:#}"
    );
    let calls = fs::read_to_string(&log).expect("program log");
    assert_eq!(
        calls.lines().count(),
        1,
        "the sync stopped after the first mirror instead of trying the next: {calls}"
    );
}
