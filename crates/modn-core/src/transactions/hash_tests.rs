//! Evidence that managed files store a BLAKE3 digest of their bytes and that
//! drift detection and crash recovery compare against that stored digest.

use super::generic_tests::{make_zip, plan};
use super::install::Fault;
use super::journal::{lock_store, temp_path};
use super::NativeInjectorPolicy;
use crate::integrity::{owned_file_checks, FileRoots};
use crate::scan::IntegrityStatus;
use crate::state::Store;
use std::fs;
use std::path::PathBuf;
use tempfile::{tempdir, TempDir};

const ONE: &[u8] = b"original";
const TWO: &[u8] = b"second";

struct Lab {
    _dir: TempDir,
    store: Store,
    game: PathBuf,
    archive: PathBuf,
}

impl Lab {
    fn new(entries: &[(&str, &[u8])]) -> Self {
        let dir = tempdir().unwrap();
        let game = dir.path().join("game");
        fs::create_dir_all(game.join("bin")).unwrap();
        fs::write(game.join("bin/TestGame.exe"), b"game-exe").unwrap();
        let game = game.canonicalize().unwrap();
        let store = Store::open(&dir.path().join("state/state.sqlite")).unwrap();
        let archive = dir.path().join("package.zip");
        make_zip(&archive, entries);
        Self {
            _dir: dir,
            store,
            game,
            archive,
        }
    }

    fn install(&mut self) -> super::ManagedPackage {
        let proposal = plan(&self.game, &self.archive);
        self.store
            .install_operations_zip(&self.game, &self.archive, &proposal)
            .unwrap()
    }

    /// Interrupts a two-file install after the first destination link.
    fn crash_after_one_file(&mut self) {
        let proposal = plan(&self.game, &self.archive);
        let guard = lock_store(&self.store).unwrap();
        let error = self
            .store
            .install_locked(
                &self.game,
                &self.archive,
                &proposal,
                Some(Fault::CrashAfter(1)),
                &[],
                None,
                NativeInjectorPolicy::Refuse,
                None,
            )
            .unwrap_err();
        drop(guard);
        assert!(
            error.to_string().contains("injected interruption"),
            "{error:#}"
        );
        assert_eq!(self.store.pending().unwrap().len(), 1);
    }

    fn transaction_id(&self) -> String {
        self.store
            .connection
            .query_row("SELECT id FROM pending_transactions", [], |row| row.get(0))
            .unwrap()
    }

    fn pending_files(&self) -> Vec<(i64, String, String, i64)> {
        let mut statement = self
            .store
            .connection
            .prepare("SELECT ord, relative_path, blake3, bytes FROM pending_files ORDER BY ord")
            .unwrap();
        statement
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    /// The check that doctor, `launch` and inventory run for owned files.
    fn status(&self) -> IntegrityStatus {
        let roots = FileRoots {
            game: &self.game,
            content: None,
            content_unavailable: IntegrityStatus::Missing,
        };
        let packages = self.store.installed(&self.game).unwrap();
        let (checks, _) = owned_file_checks(&self.store, &roots, packages);
        assert_eq!(checks.len(), 1);
        checks[0].status
    }
}

fn column_names(store: &Store, table: &str) -> Vec<String> {
    let mut statement = store
        .connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .unwrap();
    statement
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn is_lowercase_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[test]
fn managed_install_stores_the_blake3_digest_of_the_installed_bytes() {
    let mut lab = Lab::new(&[("pack/one.bin", ONE)]);
    lab.install();
    let (stored, bytes): (String, i64) = lab
        .store
        .connection
        .query_row(
            "SELECT blake3, bytes FROM managed_files WHERE relative_path=?1",
            ["data/mods/one.bin"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(stored, blake3::hash(ONE).to_hex().to_string());
    assert!(is_lowercase_hex(&stored), "{stored}");
    assert_eq!(bytes as usize, ONE.len());
    for table in ["managed_files", "pending_files"] {
        let columns = column_names(&lab.store, table);
        assert!(
            columns.iter().any(|column| column == "blake3"),
            "{table}: {columns:?}"
        );
        assert!(
            !columns.iter().any(|column| column == "sha256"),
            "{table}: {columns:?}"
        );
    }
}

#[test]
fn integrity_check_detects_drift_by_the_stored_blake3() {
    let mut lab = Lab::new(&[("pack/one.bin", ONE)]);
    lab.install();
    assert_eq!(lab.status(), IntegrityStatus::Intact);

    let dest = lab.game.join("data/mods/one.bin");
    // Same size as `ONE`, so only the digest can reveal the change.
    fs::write(&dest, b"originaL").unwrap();
    assert_eq!(fs::metadata(&dest).unwrap().len(), ONE.len() as u64);
    assert_eq!(lab.status(), IntegrityStatus::Modified);

    fs::write(&dest, ONE).unwrap();
    assert_eq!(lab.status(), IntegrityStatus::Intact);
}

#[test]
fn crashed_install_records_staged_blake3_and_recovers_matching_files() {
    let mut lab = Lab::new(&[("pack/one.bin", ONE), ("pack/two.bin", TWO)]);
    lab.crash_after_one_file();
    let id = lab.transaction_id();
    let rows = lab.pending_files();
    assert_eq!(rows.len(), 2);

    let staging = lab.store.state_root.join(&id);
    let mut recorded = Vec::new();
    for (ordinal, relative, blake3, bytes) in &rows {
        let staged = fs::read(staging.join(format!("{ordinal}.bin"))).unwrap();
        assert_eq!(blake3, &blake3::hash(&staged).to_hex().to_string());
        assert!(is_lowercase_hex(blake3), "{blake3}");
        assert_eq!(*bytes as usize, staged.len());
        assert_eq!(
            relative,
            &format!("data/mods/{}", relative.rsplit('/').next().unwrap())
        );
        recorded.push(blake3.clone());
    }
    recorded.sort();
    let mut expected = vec![
        blake3::hash(ONE).to_hex().to_string(),
        blake3::hash(TWO).to_hex().to_string(),
    ];
    expected.sort();
    assert_eq!(recorded, expected);

    let dest = lab.game.join(&rows[0].1);
    let temp = temp_path(&dest, &id, 0).unwrap();
    assert_eq!(
        fs::read(&temp).unwrap(),
        fs::read(staging.join("0.bin")).unwrap()
    );

    assert_eq!(lab.store.recover().unwrap(), 1);
    assert!(!dest.exists());
    assert!(!temp.exists());
    assert!(!lab.game.join(&rows[1].1).exists());
    assert!(!staging.exists());
    assert!(lab.store.pending().unwrap().is_empty());
}

#[test]
fn recovery_refuses_a_witness_that_differs_from_the_pending_blake3() {
    let mut lab = Lab::new(&[("pack/one.bin", ONE), ("pack/two.bin", TWO)]);
    lab.crash_after_one_file();
    let id = lab.transaction_id();
    let rows = lab.pending_files();
    let dest = lab.game.join(&rows[0].1);
    let temp = temp_path(&dest, &id, 0).unwrap();

    // Same length as `ONE`, so recovery must compare the digest.
    fs::write(&temp, b"ORIGINAL").unwrap();
    assert_ne!(blake3::hash(b"ORIGINAL").to_hex().to_string(), rows[0].2);

    let error = lab.store.recover().unwrap_err();
    assert!(
        error.to_string().contains("changed or incomplete witness"),
        "{error:#}"
    );
    assert_eq!(fs::read(&temp).unwrap(), b"ORIGINAL");
    assert_eq!(fs::read(&dest).unwrap(), b"ORIGINAL");
    assert_eq!(lab.store.pending().unwrap().len(), 1);
}
