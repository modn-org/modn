//! A stalled catalog mirror times out and the next mirror serves.

use super::CatalogHost;
use crate::buckets::git::tests as fixtures;
use crate::buckets::git::GitCatalogSource;
use crate::buckets::BucketStore;
use modn_core::manager_roots::ManagerRoots;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

/// Write a stand-in Git program that reports the real low-speed abort for one
/// remote and runs the real Git for every other call. Every call logs its
/// arguments, so a test can check the shared low-speed flags.
fn stalling_program(root: &Path, remote: &str, log: &Path) -> PathBuf {
    let marker = Path::new(remote)
        .file_name()
        .expect("remote directory name")
        .to_string_lossy();
    let marker = marker.replace('\\', "/");
    let log = log.to_string_lossy().replace('\\', "/");
    let real = real_git();
    let real = real.to_string_lossy().replace('\\', "/");
    let directory = root.join("stalling-git");
    fs::create_dir_all(&directory).expect("program directory");
    if cfg!(windows) {
        let program = directory.join("git.bat");
        fs::write(
            &program,
            format!(
                "@echo off\r\necho %* >> \"{log}\"\r\necho %* | findstr /C:\"{marker}\" >nul\r\nif errorlevel 1 goto real\r\necho fatal: unable to access 'http://127.0.0.1/': Operation too slow. Less than 1 bytes/sec transferred the last 1 seconds 1>&2\r\nexit /b 128\r\n:real\r\n\"{real}\" %*\r\n"
            ),
        )
        .expect("program");
        program
    } else {
        let program = directory.join("git");
        fs::write(
            &program,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"{log}\"\ncase \"$*\" in\n  *{marker}*)\n    echo \"fatal: unable to access 'http://127.0.0.1/': Operation too slow. Less than 1 bytes/sec transferred the last 1 seconds\" >&2\n    exit 128\n    ;;\n  *)\n    exec \"{real}\" \"$@\"\n    ;;\nesac\n"
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

/// The real Git program on PATH, called through by the stalling stub.
fn real_git() -> PathBuf {
    let name = if cfg!(windows) { "git.exe" } else { "git" };
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
        .expect("the fixture needs Git on PATH")
}

/// The test program seam for this thread, removed when the test ends.
struct Program(Option<OsString>);

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

/// A registered catalog whose primary stalls and whose stored mirror can serve.
struct Registered {
    temp: tempfile::TempDir,
    store: BucketStore,
    primary: GitCatalogSource,
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
    let store = BucketStore::new(&ManagerRoots::new(&state, None));
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
        primary: fixtures::source(&primary),
        mirror: mirror_source,
        _primary: primary,
        _mirror: mirror,
    }
}

#[test]
fn a_stalled_first_mirror_times_out_and_the_next_mirror_serves() {
    let fixture = registered();
    let log = fixture.temp.path().join("calls.log");
    let program = stalling_program(fixture.temp.path(), &fixture.primary.remote, &log);
    let _program = Program::install(&program);

    let synced = fixture
        .store
        .sync_git_with_signer("community", None)
        .expect("the second mirror serves the catalog");
    assert_eq!(
        synced.trace.served_by,
        CatalogHost::from(&fixture.mirror),
        "the mirror served the catalog"
    );
    assert_eq!(
        synced.trace.failed_sources.len(),
        1,
        "one mirror failed: {:?}",
        synced.trace.failed_sources
    );
    assert!(
        synced.trace.failed_sources[0].contains("timed out"),
        "the stalled mirror is recorded as a timeout: {:?}",
        synced.trace.failed_sources
    );
    let calls = fs::read_to_string(&log).expect("program log");
    assert!(
        calls
            .lines()
            .any(|line| line.contains("http.lowSpeedLimit=1")),
        "the catalog clone omits the shared low-speed limit: {calls}"
    );
    assert!(
        calls
            .lines()
            .any(|line| line.contains("http.lowSpeedTime=")),
        "the catalog clone omits the shared low-speed time: {calls}"
    );
}
