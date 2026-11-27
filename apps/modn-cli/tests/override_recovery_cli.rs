//! CLI contract for recovering an interrupted install that overrides a
//! lower-priority managed file.

#[path = "support/main_bucket.rs"]
mod main_bucket;

use serde_json::Value;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::{tempdir, TempDir};
use zip::write::SimpleFileOptions;

const SHARED: &str = "archive/pc/mod/override.archive";
const OLD: &[u8] = b"first-bytes";
const NEW: &[u8] = b"second-bytes";

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
                "{policy}\nlocal mapping = {{}}\nfunction policy.plan(files)\n  local operations = {{}}\n  for _, file in ipairs(files) do\n    if file.source == 'pack/shared.plugin' then\n      operations[#operations + 1] = {{ source = file.source, destination = '{SHARED}', kind = 'legacy-archive', package_root = '' }}\n    end\n  end\n  return {{ schema_version = 1, operations = operations, blockers = {{}} }}\nend\nreturn policy\n"
            ),
        )
        .expect("module");
        for (name, bytes) in [("first", OLD), ("second", NEW)] {
            let mut zip = zip::ZipWriter::new(
                File::create(temp.path().join(format!("{name}.zip"))).expect("zip"),
            );
            zip.start_file("pack/shared.plugin", SimpleFileOptions::default())
                .expect("entry");
            zip.write_all(bytes).expect("bytes");
            zip.finish().expect("finish");
        }
        Self {
            state: temp.path().join("state"),
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

    fn recover(&self) -> Output {
        self.modn(&["recover", "--yes"])
    }

    fn list(&self) -> Value {
        let game = self.game.to_string_lossy().into_owned();
        let output = self.modn(&["list", "--path", &game]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("list JSON")
    }

    fn path(&self) -> PathBuf {
        self.game.join(SHARED)
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

fn digest(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

struct Interrupted {
    temp: PathBuf,
    backup: PathBuf,
}

impl Interrupted {
    fn new(world: &World, status: &str) -> Self {
        let id = if status == "committed" {
            "txn-override-committed".to_owned()
        } else {
            "txn-override-crashed".to_owned()
        };
        let dest = world.path();
        let temp = dest
            .parent()
            .expect("destination parent")
            .join(format!(".modn-txn-{id}-0.tmp"));
        let backup = dest
            .parent()
            .expect("destination parent")
            .join(format!(".modn-txn-{id}-0.old"));
        let connection = world.database();
        connection
            .execute(
                "INSERT INTO pending_transactions
                 (id, action, status, game_path, package_id, archive_blake3)
                 VALUES (?1, 'install', ?2, ?3, ?1, 'archive')",
                rusqlite::params![id, status, world.game.to_string_lossy().as_ref()],
            )
            .expect("pending transaction");
        connection
            .execute(
                "INSERT INTO pending_files
                 (transaction_id, ord, root, relative_path, blake3, bytes)
                 VALUES (?1, 0, 'game', ?2, ?3, ?4)",
                rusqlite::params![id, SHARED, digest(NEW), NEW.len() as i64],
            )
            .expect("pending file");
        connection
            .execute(
                "INSERT INTO pending_overrides
                 (transaction_id, ord, root, relative_path, package_id, blake3, bytes)
                 VALUES (?1, 0, 'game', ?2, 'previous', ?3, ?4)",
                rusqlite::params![id, SHARED, digest(OLD), OLD.len() as i64],
            )
            .expect("pending override");
        Self { temp, backup }
    }

    /// Leave the state of a crash after the new file was placed but before the
    /// manager commit: `.old` and `.tmp` witnesses beside the new destination.
    fn placed(world: &World) -> Self {
        let interrupted = Self::new(world, "pending");
        let dest = world.path();
        fs::hard_link(&dest, &interrupted.backup).expect("old witness");
        fs::remove_file(&dest).expect("remove old bytes");
        fs::write(&interrupted.temp, NEW).expect("new witness");
        fs::hard_link(&interrupted.temp, &dest).expect("placed new bytes");
        interrupted
    }

    fn witnesses_exist(&self) -> bool {
        self.temp.is_file() && self.backup.is_file()
    }
}

fn pending_count(world: &World) -> i64 {
    world
        .database()
        .query_row("SELECT COUNT(*) FROM pending_transactions", [], |row| {
            row.get(0)
        })
        .expect("pending count")
}

#[test]
fn cli_override_recovery_restores_old_bytes_before_commit() {
    let world = World::new();
    let first = world.install("first");
    assert!(first.status.success(), "{first:?}");
    let first_id = package_id(&first);
    let interrupted = Interrupted::placed(&world);
    assert_eq!(fs::read(world.path()).expect("new bytes"), NEW);

    let recovered = world.recover();
    assert!(
        recovered.status.success(),
        "{}",
        String::from_utf8_lossy(&recovered.stderr)
    );
    assert_eq!(json(&recovered)["recovered"], 1);
    assert_eq!(fs::read(world.path()).expect("old bytes"), OLD);
    assert!(!interrupted.witnesses_exist(), "witnesses removed");
    assert_eq!(pending_count(&world), 0);

    let list = world.list();
    let packages = list.as_array().expect("list array");
    assert_eq!(packages.len(), 1, "{list}");
    assert_eq!(packages[0]["id"], first_id);
}

#[test]
fn cli_override_recovery_restores_a_missing_destination_from_the_witness() {
    let world = World::new();
    let first = world.install("first");
    assert!(first.status.success(), "{first:?}");
    let interrupted = Interrupted::placed(&world);
    fs::remove_file(world.path()).expect("interrupted before placement");

    let recovered = world.recover();
    assert!(
        recovered.status.success(),
        "{}",
        String::from_utf8_lossy(&recovered.stderr)
    );
    assert_eq!(json(&recovered)["recovered"], 1);
    assert_eq!(fs::read(world.path()).expect("old bytes"), OLD);
    assert!(!interrupted.witnesses_exist(), "witnesses removed");
    assert_eq!(world.list().as_array().expect("list").len(), 1);
}

#[test]
fn cli_override_recovery_keeps_a_committed_override() {
    let world = World::new();
    let first = world.install("first");
    assert!(first.status.success(), "{first:?}");
    let second = world.install("second");
    assert!(second.status.success(), "{second:?}");
    let second_id = package_id(&second);
    let interrupted = Interrupted::new(&world, "committed");
    fs::write(&interrupted.backup, OLD).expect("old witness");
    fs::write(&interrupted.temp, NEW).expect("new witness");
    assert_eq!(fs::read(world.path()).expect("new bytes"), NEW);

    let recovered = world.recover();
    assert!(
        recovered.status.success(),
        "{}",
        String::from_utf8_lossy(&recovered.stderr)
    );
    assert_eq!(json(&recovered)["recovered"], 1);
    assert_eq!(fs::read(world.path()).expect("new bytes"), NEW);
    assert!(!interrupted.witnesses_exist(), "witnesses removed");
    assert_eq!(pending_count(&world), 0);
    let list = world.list();
    let packages = list.as_array().expect("list array");
    assert_eq!(packages.len(), 2, "{list}");
    assert!(
        packages.iter().any(|package| package["id"] == second_id),
        "{list}"
    );
}

#[test]
fn cli_override_recovery_refuses_a_user_edited_destination() {
    let world = World::new();
    let first = world.install("first");
    assert!(first.status.success(), "{first:?}");
    let interrupted = Interrupted::placed(&world);
    fs::write(world.path(), b"user-edit").expect("user edit");

    let refused = world.recover();
    assert!(!refused.status.success(), "an edited file must refuse");
    assert_eq!(error_code(&refused), "CONFLICT");
    let message = String::from_utf8_lossy(&refused.stderr);
    assert!(message.contains("destination changed"), "{message}");
    assert_eq!(fs::read(world.path()).expect("user bytes"), b"user-edit");
    assert!(interrupted.witnesses_exist(), "witnesses preserved");
    assert_eq!(pending_count(&world), 1);
    assert_eq!(world.list().as_array().expect("list").len(), 1);
}

#[test]
fn cli_override_recovery_is_idempotent() {
    let world = World::new();
    assert!(world.install("first").status.success());
    let interrupted = Interrupted::placed(&world);

    assert!(world.recover().status.success());
    let second = world.recover();
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(json(&second)["recovered"], 0);
    assert_eq!(fs::read(world.path()).expect("old bytes"), OLD);
    assert!(!interrupted.witnesses_exist());
}
