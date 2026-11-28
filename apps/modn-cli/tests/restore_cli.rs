//! CLI contract for uninstalling a winner and restoring the next claimant.

#[path = "support/main_bucket.rs"]
mod main_bucket;

use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::{tempdir, TempDir};
use zip::write::SimpleFileOptions;

const SHARED: &str = "archive/pc/mod/override.archive";
const FIRST: &[u8] = b"first-bytes";
const SECOND: &[u8] = b"second-bytes";
const ENTRY: &str = "pack/shared.plugin";

struct World {
    temp: TempDir,
    game: PathBuf,
    state: PathBuf,
    modules: PathBuf,
}

impl World {
    fn new() -> Self {
        let temp = tempdir().expect("temp");
        main_bucket::seed(temp.path());
        let game = temp.path().join("world/game");
        fs::create_dir_all(game.join("bin/x64")).expect("bin");
        fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").expect("exe");
        let modules = temp.path().join("modules");
        let module = modules.join("cyberpunk-2077");
        let bundled =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main/games/cyberpunk-2077");
        main_bucket::copy_main_module(&bundled, &module);
        let code = fs::read_to_string(module.join("module.lua"))
            .expect("module")
            .replace("\r\n", "\n");
        let policy = code
            .strip_suffix("return policy\n")
            .expect("module suffix")
            .to_owned();
        fs::write(
            module.join("module.lua"),
            format!(
                "{policy}\nlocal mapping = {{}}\nfunction policy.plan(files)\n  local operations = {{}}\n  for _, file in ipairs(files) do\n    if file.source == '{ENTRY}' then\n      operations[#operations + 1] = {{ source = file.source, destination = '{SHARED}', kind = 'legacy-archive', package_root = '' }}\n    end\n  end\n  return {{ schema_version = 1, operations = operations, blockers = {{}} }}\nend\nreturn policy\n"
            ),
        )
        .expect("module");
        for (name, bytes) in [("first", FIRST), ("second", SECOND)] {
            let mut zip = zip::ZipWriter::new(
                File::create(temp.path().join(format!("{name}.zip"))).expect("zip"),
            );
            zip.start_file(ENTRY, SimpleFileOptions::default())
                .expect("entry");
            zip.write_all(bytes).expect("bytes");
            zip.finish().expect("finish");
        }
        let state = temp.path().join("state");
        let cache = state.join("cache");
        fs::create_dir_all(&cache).expect("cache");
        for name in ["first", "second"] {
            fs::copy(
                temp.path().join(format!("{name}.zip")),
                cache.join(format!("cyberpunk-2077#{name}#1.0#0000000.zip")),
            )
            .expect("cache entry");
        }
        Self {
            state,
            game,
            modules,
            temp,
        }
    }

    fn archive(&self, name: &str) -> PathBuf {
        self.temp.path().join(format!("{name}.zip"))
    }

    fn modn(&self, args: &[&str]) -> Output {
        Command::new(main_bucket::modn_binary())
            .env("LOCALAPPDATA", self.temp.path())
            .env("APPDATA", self.temp.path())
            .env("MODN_APPDATA", self.temp.path())
            .env("MODN_GAME_MODULES_DIR", &self.modules)
            .args(["--game", "cyberpunk-2077", "--json", "--state-dir"])
            .arg(&self.state)
            .args(args)
            .output()
            .expect("modn")
    }

    fn install(&self, name: &str) -> Output {
        let archive = self.archive(name);
        let game = self.game.to_string_lossy().into_owned();
        self.modn(&[
            "install",
            &archive.to_string_lossy(),
            "--yes",
            "--path",
            &game,
        ])
    }

    fn uninstall(&self, id: &str) -> Output {
        let game = self.game.to_string_lossy().into_owned();
        self.modn(&["uninstall", id, "--yes", "--path", &game])
    }

    fn dry_run(&self, id: &str) -> Output {
        let game = self.game.to_string_lossy().into_owned();
        self.modn(&["uninstall", id, "--dry-run", "--path", &game])
    }

    fn recover(&self) -> Output {
        self.modn(&["recover", "--yes"])
    }

    fn list(&self) -> String {
        let game = self.game.to_string_lossy().into_owned();
        let output = self.modn(&["list", "--path", &game]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn path(&self) -> PathBuf {
        self.game.join(SHARED)
    }

    fn snapshot(&self) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
        snapshot(&self.game)
    }

    fn database(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.state.join("state.sqlite")).expect("manager state")
    }
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("json")
}

fn package_id(output: &Output) -> String {
    json(output)["id"].as_str().expect("package id").to_owned()
}

fn error_code(output: &Output) -> String {
    let value: Value = serde_json::from_slice(&output.stderr).expect("JSON error");
    value["error"].as_str().expect("error code").to_owned()
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let mut entries = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries_at) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries_at.flatten() {
            let path = entry.path();
            let relative = path.strip_prefix(root).expect("relative").to_path_buf();
            if fs::symlink_metadata(&path).expect("metadata").is_dir() {
                pending.push(path);
                entries.insert(relative, None);
            } else {
                entries.insert(relative, Some(fs::read(&path).expect("bytes")));
            }
        }
    }
    entries
}

fn installed(world: &World) -> Value {
    let game = world.game.to_string_lossy().into_owned();
    let output = world.modn(&["list", "--path", &game]);
    assert!(output.status.success());
    serde_json::from_slice(&output.stdout).expect("list")
}

#[test]
fn restore_uninstall_of_the_winner_restores_the_first_mod() {
    let world = World::new();
    let first = world.install("first");
    assert!(first.status.success(), "{first:?}");
    let first_id = package_id(&first);
    let second = world.install("second");
    assert!(second.status.success(), "{second:?}");
    let second_id = package_id(&second);
    assert_eq!(fs::read(world.path()).expect("bytes"), SECOND);

    let removed = world.uninstall(&second_id);
    assert!(
        removed.status.success(),
        "{}",
        String::from_utf8_lossy(&removed.stderr)
    );
    assert_eq!(fs::read(world.path()).expect("bytes"), FIRST);
    let packages = installed(&world);
    let packages = packages.as_array().expect("list array");
    assert_eq!(packages.len(), 1, "{packages:?}");
    assert_eq!(packages[0]["id"], first_id);
}

#[test]
fn restore_reverse_order_leaves_the_game_clean() {
    let world = World::new();
    let first = world.install("first");
    assert!(first.status.success(), "{first:?}");
    let first_id = package_id(&first);
    let second = world.install("second");
    assert!(second.status.success(), "{second:?}");
    let second_id = package_id(&second);

    assert!(world.uninstall(&first_id).status.success());
    assert_eq!(fs::read(world.path()).expect("bytes"), SECOND);
    let removed = world.uninstall(&second_id);
    assert!(removed.status.success(), "{removed:?}");
    assert!(!world.path().exists());
    assert_eq!(installed(&world).as_array().expect("list").len(), 0);
}

#[test]
fn restore_missing_cache_refuses_without_change() {
    let world = World::new();
    assert!(world.install("first").status.success());
    let second = world.install("second");
    assert!(second.status.success(), "{second:?}");
    let second_id = package_id(&second);
    fs::remove_dir_all(world.state.join("cache")).expect("remove cache");
    let before = world.snapshot();
    let list_before = world.list();

    let refused = world.uninstall(&second_id);
    assert!(!refused.status.success(), "a missing archive must refuse");
    assert_eq!(error_code(&refused), "PROFILE_ARCHIVE_MISSING");
    let message = String::from_utf8_lossy(&refused.stderr);
    assert!(message.contains("PROFILE_ARCHIVE_MISSING:"), "{message}");
    assert!(message.contains("modn cache show"), "{message}");
    assert_eq!(world.snapshot(), before);
    assert_eq!(world.list(), list_before);
    assert_eq!(fs::read(world.path()).expect("bytes"), SECOND);
}

#[test]
fn restore_dry_run_reports_the_restore_and_changes_nothing() {
    let world = World::new();
    assert!(world.install("first").status.success());
    let second = world.install("second");
    assert!(second.status.success(), "{second:?}");
    let second_id = package_id(&second);
    let before = world.snapshot();

    let dry = world.dry_run(&second_id);
    assert!(dry.status.success(), "{dry:?}");
    let preview = json(&dry);
    let packages = preview["packages"].as_array().expect("packages");
    let restores = packages[0]["restores"].as_array().expect("restores");
    assert_eq!(restores.len(), 1, "{preview}");
    assert_eq!(restores[0]["destination"], SHARED);
    assert_eq!(world.snapshot(), before);
    assert_eq!(fs::read(world.path()).expect("bytes"), SECOND);
}

#[test]
fn restore_drifted_winner_refuses_with_profile_drift() {
    let world = World::new();
    assert!(world.install("first").status.success());
    let second = world.install("second");
    assert!(second.status.success(), "{second:?}");
    let second_id = package_id(&second);
    fs::write(world.path(), b"user-edit").expect("edit");
    let list_before = world.list();

    let refused = world.uninstall(&second_id);
    assert!(!refused.status.success());
    assert_eq!(error_code(&refused), "PROFILE_DRIFT");
    let message = String::from_utf8_lossy(&refused.stderr);
    assert!(message.contains("PROFILE_DRIFT:"), "{message}");
    assert!(message.contains("modn doctor"), "{message}");
    assert_eq!(fs::read(world.path()).expect("bytes"), b"user-edit");
    assert_eq!(world.list(), list_before);
}

#[test]
fn restore_recover_yes_rolls_back_an_interrupted_restore() {
    let world = World::new();
    let first = world.install("first");
    assert!(first.status.success(), "{first:?}");
    let first_id = package_id(&first);
    let second = world.install("second");
    assert!(second.status.success(), "{second:?}");
    let second_id = package_id(&second);
    let interrupted = Interrupted::placed(&world, &first_id, &second_id);
    assert_eq!(fs::read(world.path()).expect("restored"), FIRST);

    let recovered = world.recover();
    assert!(
        recovered.status.success(),
        "{}",
        String::from_utf8_lossy(&recovered.stderr)
    );
    assert_eq!(json(&recovered)["recovered"], 1);
    assert_eq!(fs::read(world.path()).expect("winner"), SECOND);
    assert!(!interrupted.witnesses_exist(), "witnesses removed");
    assert_eq!(
        world
            .database()
            .query_row("SELECT COUNT(*) FROM pending_transactions", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("pending"),
        0
    );
    assert_eq!(installed(&world).as_array().expect("list").len(), 2);
}

#[test]
fn restore_undo_of_a_winning_install_refuses() {
    let world = World::new();
    assert!(world.install("first").status.success());
    let second = world.install("second");
    assert!(second.status.success(), "{second:?}");
    let operation = json(&second)["operation_id"]
        .as_str()
        .expect("operation id")
        .to_owned();
    let before = world.snapshot();

    let game = world.game.to_string_lossy().into_owned();
    let refused = world.modn(&["undo", &operation, "--yes", "--path", &game]);
    assert!(!refused.status.success(), "undo must refuse");
    assert_eq!(
        error_code(&refused),
        "CONFLICT",
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );
    let message = String::from_utf8_lossy(&refused.stderr);
    assert!(
        message.contains("cannot restore the next claimant"),
        "{message}"
    );
    assert_eq!(world.snapshot(), before);
}

struct Interrupted {
    temp: PathBuf,
    restore: PathBuf,
}

impl Interrupted {
    /// Leave the state of a crash after the claimant's bytes were placed but
    /// before the manager commit.
    fn placed(world: &World, first_id: &str, second_id: &str) -> Self {
        let id = "txn-restore-crashed";
        let dest = world.path();
        let temp = dest
            .parent()
            .expect("destination parent")
            .join(format!(".modn-txn-{id}-0.tmp"));
        let restore = dest
            .parent()
            .expect("destination parent")
            .join(format!(".modn-txn-{id}-0.restore"));
        let connection = world.database();
        connection
            .execute(
                "INSERT INTO pending_transactions
                 (id, action, status, game_path, package_id, archive_blake3)
                 VALUES (?1, 'uninstall', 'pending', ?2, ?3, NULL)",
                rusqlite::params![id, world.game.to_string_lossy().as_ref(), second_id],
            )
            .expect("pending transaction");
        connection
            .execute(
                "INSERT INTO pending_files
                 (transaction_id, ord, root, relative_path, blake3, bytes)
                 VALUES (?1, 0, 'game', ?2, ?3, ?4)",
                rusqlite::params![id, SHARED, digest(SECOND), SECOND.len() as i64],
            )
            .expect("pending file");
        connection
            .execute(
                "INSERT INTO pending_restores
                 (transaction_id, ord, package_id, archive_entry, blake3, bytes)
                 VALUES (?1, 0, ?2, ?3, ?4, ?5)",
                rusqlite::params![id, first_id, ENTRY, digest(FIRST), FIRST.len() as i64],
            )
            .expect("pending restore");
        fs::write(&temp, SECOND).expect("old winner witness");
        fs::write(&restore, FIRST).expect("restored witness");
        fs::write(&dest, FIRST).expect("restored bytes in place");
        Self { temp, restore }
    }

    fn witnesses_exist(&self) -> bool {
        self.temp.is_file() && self.restore.is_file()
    }
}

fn digest(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
