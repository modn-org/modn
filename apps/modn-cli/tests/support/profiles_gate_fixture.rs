//! Helpers for the Milestone 20 end-to-end CLI gate.
//!
//! Every helper uses the disposable [`ProfileFixture`]: the game directory and
//! the manager state live in a temporary folder, and each archive is copied
//! into the fixture cache so priority, switch and recovery can restore bytes.

use crate::profile_fixture::ProfileFixture;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;

/// The path both gate mods claim.
pub const SHARED: &str = "archive/pc/mod/shared.archive";
/// A path only mod A writes.
pub const ALPHA_UNIQUE: &str = "archive/pc/mod/alpha-unique.archive";
/// A path only mod B writes.
pub const BETA_UNIQUE: &str = "archive/pc/mod/beta-unique.archive";
/// A path only mod C writes.
pub const GAMMA: &str = "archive/pc/mod/gamma.archive";
/// A file no package manages.
pub const USER: &str = "archive/pc/mod/user.cfg";

pub const ALPHA_SHARED: &[u8] = b"alpha-shared";
pub const ALPHA_UNIQUE_BYTES: &[u8] = b"alpha-unique";
pub const BETA_SHARED: &[u8] = b"beta-shared";
pub const BETA_UNIQUE_BYTES: &[u8] = b"beta-unique";
pub const GAMMA_BYTES: &[u8] = b"gamma";
pub const USER_BYTES: &[u8] = b"user";

/// Installs one local archive, caches its bytes and returns the package id.
pub fn install(fixture: &ProfileFixture, name: &str, files: &[(&str, &[u8])]) -> String {
    let archive = fixture
        .state()
        .parent()
        .expect("fixture root")
        .join("archives")
        .join(format!("{name}.zip"));
    let file = File::create(&archive).expect("archive");
    let mut zip = zip::ZipWriter::new(file);
    for (entry, bytes) in files {
        zip.start_file(*entry, SimpleFileOptions::default())
            .expect("archive entry");
        zip.write_all(bytes).expect("archive payload");
    }
    zip.finish().expect("archive finish");
    let report = fixture.json(&[
        "install",
        archive.to_str().expect("archive path"),
        "--path",
        fixture.game().to_str().expect("game path"),
        "--yes",
    ]);
    let cache = fixture.state().join("cache");
    fs::create_dir_all(&cache).expect("cache directory");
    fs::copy(
        &archive,
        cache.join(format!("cyberpunk-2077#{name}#1.0#0000000.zip")),
    )
    .expect("cache entry");
    report["id"].as_str().expect("package id").to_owned()
}

/// Installs mod A: the shared path plus one unique file.
pub fn install_alpha(fixture: &ProfileFixture) -> String {
    install(
        fixture,
        "alpha",
        &[(SHARED, ALPHA_SHARED), (ALPHA_UNIQUE, ALPHA_UNIQUE_BYTES)],
    )
}

/// Installs mod B: the shared path plus one unique file.
pub fn install_beta(fixture: &ProfileFixture) -> String {
    install(
        fixture,
        "beta",
        &[(SHARED, BETA_SHARED), (BETA_UNIQUE, BETA_UNIQUE_BYTES)],
    )
}

/// Installs mod C into the active profile at a distinct path.
pub fn install_gamma(fixture: &ProfileFixture) -> String {
    install(fixture, "gamma", &[(GAMMA, GAMMA_BYTES)])
}

/// Creates the inactive `second` profile.
pub fn create_second(fixture: &ProfileFixture) {
    fixture.json(&["profile", "create", "second"]);
}

/// Uninstalls one package from the selected installation.
pub fn uninstall(fixture: &ProfileFixture, package: &str) -> Value {
    fixture.json(&[
        "uninstall",
        package,
        "--yes",
        "--path",
        fixture.game().to_str().expect("game path"),
    ])
}

/// Every file and directory below the game installation.
pub fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let mut entries = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).expect("game directory") {
            let path = entry.expect("entry").path();
            let relative = path
                .strip_prefix(root)
                .expect("relative path")
                .to_path_buf();
            if fs::symlink_metadata(&path).expect("metadata").is_dir() {
                pending.push(path);
                entries.insert(relative, None);
            } else {
                entries.insert(relative, Some(fs::read(&path).expect("file bytes")));
            }
        }
    }
    entries
}

/// The names `profile list --json` marks active.
pub fn active_profiles(fixture: &ProfileFixture) -> Vec<String> {
    let list = fixture.profile_list();
    list["profiles"]
        .as_array()
        .expect("profiles")
        .iter()
        .filter(|profile| profile["active"] == true)
        .map(|profile| profile["name"].as_str().expect("name").to_owned())
        .collect()
}

/// Asserts that exactly one profile is active and returns its name.
pub fn assert_one_active(fixture: &ProfileFixture, step: &str) -> String {
    let active = active_profiles(fixture);
    assert_eq!(
        active.len(),
        1,
        "{step}: exactly one profile must be active: {active:?}"
    );
    active[0].clone()
}

/// Leaves a pending `default` to `second` switch that crashed after mod C's
/// bytes were placed and mod A's shared file was quarantined.
///
/// Returns the `.old` witness path.
pub fn interrupt_switch(fixture: &ProfileFixture, id: &str, alpha: &str, beta: &str) -> PathBuf {
    let shared = fixture.game().join(SHARED);
    let witness = shared
        .parent()
        .expect("shared parent")
        .join(format!(".modn-txn-{id}-1.old"));
    fs::rename(&shared, &witness).expect("quarantine witness");
    fs::write(fixture.game().join(GAMMA), GAMMA_BYTES).expect("partial write");
    let game = fixture.game().to_string_lossy().into_owned();
    let connection =
        rusqlite::Connection::open(fixture.state().join("state.sqlite")).expect("state database");
    connection
        .execute(
            "INSERT INTO pending_transactions
             (id, action, status, game_path, package_id, archive_blake3)
             VALUES (?1, 'profile-switch', 'pending', ?2, '', NULL)",
            rusqlite::params![id, game],
        )
        .expect("pending transaction");
    connection
        .execute(
            "INSERT INTO pending_profile_switches(transaction_id, game_path, from_profile, to_profile)
             VALUES (?1, ?2, 'default', 'second')",
            rusqlite::params![id, game],
        )
        .expect("pending switch");
    connection
        .execute(
            "INSERT INTO pending_files(transaction_id, ord, root, relative_path, blake3, bytes)
             VALUES (?1, 0, 'game', ?2, ?3, ?4)",
            rusqlite::params![id, GAMMA, digest(GAMMA_BYTES), GAMMA_BYTES.len() as i64],
        )
        .expect("pending write");
    for (ordinal, path, package, bytes) in [
        (1, SHARED, alpha, ALPHA_SHARED),
        (2, ALPHA_UNIQUE, alpha, ALPHA_UNIQUE_BYTES),
        (3, BETA_UNIQUE, beta, BETA_UNIQUE_BYTES),
    ] {
        connection
            .execute(
                "INSERT INTO pending_overrides
                 (transaction_id, ord, root, relative_path, package_id, blake3, bytes)
                 VALUES (?1, ?2, 'game', ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    id,
                    ordinal,
                    path,
                    package,
                    digest(bytes),
                    bytes.len() as i64
                ],
            )
            .expect("pending removal");
    }
    drop(connection);
    witness
}

/// BLAKE3 digest of one payload.
pub fn digest(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
