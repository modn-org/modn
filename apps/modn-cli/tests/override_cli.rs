//! CLI contract for installing over a lower-priority managed file.
#![allow(dead_code, reason = "shared test fixture helpers")]

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
        for name in ["first", "second"] {
            let mut zip = zip::ZipWriter::new(
                File::create(temp.path().join(format!("{name}.zip"))).expect("zip"),
            );
            zip.start_file("pack/shared.plugin", SimpleFileOptions::default())
                .expect("entry");
            zip.write_all(format!("{name}-bytes").as_bytes())
                .expect("bytes");
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

    fn dry_run(&self, name: &str) -> Output {
        let archive = self.archive(name);
        let game = self.game.to_string_lossy().into_owned();
        self.modn(&[
            "install",
            &archive.to_string_lossy(),
            "--dry-run",
            "--json",
            "--path",
            &game,
        ])
    }

    fn uninstall(&self, id: &str) -> Output {
        let game = self.game.to_string_lossy().into_owned();
        self.modn(&["uninstall", id, "--yes", "--path", &game])
    }

    fn path(&self) -> PathBuf {
        self.game.join(SHARED)
    }
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("json")
}

fn package_id(output: &Output) -> String {
    json(output)["id"].as_str().expect("package id").to_owned()
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

#[test]
fn override_install_replaces_the_lower_package_file() {
    let world = World::new();
    let first = world.install("first");
    assert!(first.status.success(), "{first:?}");
    let first_id = package_id(&first);
    let second = world.install("second");
    assert!(second.status.success(), "{second:?}");

    assert_eq!(fs::read(world.path()).expect("bytes"), b"second-bytes");
    let second_id = package_id(&second);
    assert!(world.uninstall(&first_id).status.success());
    assert_eq!(fs::read(world.path()).expect("bytes"), b"second-bytes");
    assert!(world.uninstall(&second_id).status.success());
}

#[test]
fn override_dry_run_lists_the_override_and_changes_nothing() {
    let world = World::new();
    assert!(world.install("first").status.success());
    let before = snapshot(&world.game);
    let state_before = fs::read(world.state.join("state.sqlite")).expect("db");

    let dry = world.dry_run("second");
    assert!(dry.status.success(), "{dry:?}");
    let preview = json(&dry);
    assert_eq!(preview["status"], "reviewable");
    let overrides = preview["overrides"].as_array().expect("overrides");
    assert_eq!(overrides.len(), 1, "{preview}");
    assert_eq!(overrides[0]["destination"], SHARED);
    assert!(overrides[0]["replaced_package"].is_string());

    assert_eq!(snapshot(&world.game), before);
    assert_eq!(
        fs::read(world.state.join("state.sqlite")).expect("db"),
        state_before
    );
}

#[test]
fn override_unmanaged_file_is_refused() {
    let world = World::new();
    fs::create_dir_all(world.path().parent().expect("parent")).expect("dirs");
    fs::write(world.path(), b"user-bytes").expect("user");
    let before = snapshot(&world.game);

    let refused = world.install("second");
    assert!(!refused.status.success());
    assert_eq!(fs::read(world.path()).expect("bytes"), b"user-bytes");
    assert_eq!(snapshot(&world.game), before);
}

#[test]
fn override_tampered_owned_file_is_refused_without_a_byte_of_change() {
    let world = World::new();
    assert!(world.install("first").status.success());
    fs::write(world.path(), b"user-change").expect("tamper");
    let before = snapshot(&world.game);

    let refused = world.install("second");
    assert!(!refused.status.success());
    assert_eq!(snapshot(&world.game), before);
}

#[test]
fn override_uninstall_in_install_order_restores_the_game() {
    let world = World::new();
    let start = snapshot(&world.game);
    let first = world.install("first");
    assert!(first.status.success());
    let first_id = package_id(&first);
    let second = world.install("second");
    assert!(second.status.success());
    let second_id = package_id(&second);

    assert!(world.uninstall(&first_id).status.success());
    assert_eq!(fs::read(world.path()).expect("bytes"), b"second-bytes");
    assert!(world.uninstall(&second_id).status.success());
    assert_eq!(snapshot(&world.game), start);
}

const CATALOG_SHARED: &str = "archive/pc/mod/override.archive";

/// A catalog of legacy archives that all claim one game file.
struct Catalog {
    temp: TempDir,
    game: PathBuf,
    state: PathBuf,
}

impl Catalog {
    fn new() -> Self {
        let temp = tempdir().expect("temp");
        let game = temp.path().join("game");
        fs::create_dir_all(game.join("bin/x64")).expect("bin");
        fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").expect("exe");
        let state = temp.path().join("state");
        main_bucket::seed_state(&state);
        let catalog = Self { temp, game, state };
        let added = catalog.cli(&[
            "game",
            "add",
            "cyberpunk-2077",
            "--path",
            catalog.game.to_str().expect("game"),
        ]);
        assert!(added.status.success(), "{added:?}");
        catalog
    }

    fn cli(&self, args: &[&str]) -> Output {
        main_bucket::command(env!("CARGO_BIN_EXE_modn"))
            .args(["--game", "cyberpunk-2077", "--json", "--state-dir"])
            .arg(&self.state)
            .args(args)
            .output()
            .expect("modn")
    }

    fn zip(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.temp.path().join(name);
        let mut zip = zip::ZipWriter::new(File::create(&path).expect("zip"));
        zip.start_file(CATALOG_SHARED, SimpleFileOptions::default())
            .expect("entry");
        zip.write_all(bytes).expect("bytes");
        zip.finish().expect("finish");
        path
    }

    fn digest(&self, archive: &Path) -> String {
        blake3::hash(&fs::read(archive).expect("bytes"))
            .to_hex()
            .to_string()
    }

    fn register(&self, packages: &[Value]) {
        let root = self.temp.path().join("community");
        let games = root.join("mods/cyberpunk-2077");
        fs::create_dir_all(&games).expect("dirs");
        fs::write(
            root.join("bucket.json"),
            r#"{"schema_version":2,"name":"community","title":"Community"}"#,
        )
        .expect("metadata");
        for package in packages {
            let id = package["id"].as_str().expect("id");
            fs::write(
                games.join(format!("{id}.json")),
                serde_json::to_vec(package).expect("manifest"),
            )
            .expect("manifest");
        }
        let added = self.cli(&["bucket", "add", "community", root.to_str().expect("root")]);
        assert!(added.status.success(), "{added:?}");
    }

    fn install(&self, id: &str, archive: &Path, yes: bool) -> Output {
        let reference = format!("community/cyberpunk-2077/{id}");
        let mut args = vec![
            "install",
            &reference,
            "--file",
            archive.to_str().expect("archive"),
            "--path",
            self.game.to_str().expect("game"),
        ];
        if yes {
            args.push("--yes");
        } else {
            args.push("--dry-run");
        }
        self.cli(&args)
    }

    fn path(&self) -> PathBuf {
        self.game.join(CATALOG_SHARED)
    }

    fn state_bytes(&self) -> Vec<u8> {
        fs::read(self.state.join("state.sqlite")).expect("state")
    }
}

fn catalog_manifest(id: &str, pin: &str, conflicts: &[&str]) -> Value {
    serde_json::json!({
        "schema_version": 2, "id": id, "game": "cyberpunk-2077", "name": id,
        "version": "1.0", "archive_format": "zip", "blake3": pin,
        "sources": [{"kind": "manual", "filename": format!("{id}.zip")}],
        "conflicts": conflicts,
    })
}

#[test]
fn override_catalog_install_replaces_a_managed_file() {
    let catalog = Catalog::new();
    let first = catalog.zip("first.zip", b"first-bytes");
    let second = catalog.zip("second.zip", b"second-bytes");
    catalog.register(&[
        catalog_manifest("first", &catalog.digest(&first), &[]),
        catalog_manifest("second", &catalog.digest(&second), &[]),
    ]);
    let installed = catalog.install("first", &first, true);
    assert!(installed.status.success(), "{installed:?}");
    let game_before = snapshot(&catalog.game);
    let state_before = catalog.state_bytes();

    let dry = catalog.install("second", &second, false);
    assert!(dry.status.success(), "{dry:?}");
    let preview = json(&dry);
    assert_eq!(preview["plan"]["status"], "reviewable", "{preview}");
    let overrides = preview["plan"]["overrides"].as_array().expect("overrides");
    assert_eq!(overrides.len(), 1, "{preview}");
    assert_eq!(overrides[0]["destination"], CATALOG_SHARED);
    assert!(overrides[0]["replaced_package"].is_string(), "{preview}");
    assert_eq!(snapshot(&catalog.game), game_before);
    assert_eq!(catalog.state_bytes(), state_before);

    let replaced = catalog.install("second", &second, true);
    assert!(replaced.status.success(), "{replaced:?}");
    assert_eq!(fs::read(catalog.path()).expect("bytes"), b"second-bytes");
    let first_id = json(&installed)["installed"]["id"]
        .as_str()
        .expect("first id")
        .to_owned();
    let second_id = json(&replaced)["installed"]["id"]
        .as_str()
        .expect("second id")
        .to_owned();
    let database = rusqlite::Connection::open(catalog.state.join("state.sqlite")).expect("state");
    let claims: i64 = database
        .query_row("SELECT COUNT(*) FROM file_claims", [], |row| row.get(0))
        .expect("claims");
    assert_eq!(claims, 2);
    let winner: String = database
        .query_row(
            "SELECT package_id FROM managed_files WHERE relative_path=?1",
            [CATALOG_SHARED],
            |row| row.get(0),
        )
        .expect("winner");
    assert_eq!(winner, second_id);
    let shadowed: String = database
        .query_row(
            "SELECT package_id FROM file_claims WHERE package_id=?1",
            [&first_id],
            |row| row.get(0),
        )
        .expect("shadowed claim");
    assert_eq!(shadowed, first_id);
}

#[test]
fn override_declared_conflict_is_refused_without_change() {
    let catalog = Catalog::new();
    let first = catalog.zip("first.zip", b"first-bytes");
    let second = catalog.zip("second.zip", b"second-bytes");
    catalog.register(&[
        catalog_manifest("first", &catalog.digest(&first), &[]),
        catalog_manifest("second", &catalog.digest(&second), &["first"]),
    ]);
    let installed = catalog.install("first", &first, true);
    assert!(installed.status.success(), "{installed:?}");
    let game_before = snapshot(&catalog.game);
    let state_before = catalog.state_bytes();

    let refused = catalog.install("second", &second, true);
    assert!(!refused.status.success());
    let error: Value = serde_json::from_slice(&refused.stderr).expect("error json");
    assert_eq!(error["error"], "CONFLICT", "{error}");
    assert_eq!(fs::read(catalog.path()).expect("bytes"), b"first-bytes");
    assert_eq!(snapshot(&catalog.game), game_before);
    assert_eq!(catalog.state_bytes(), state_before);
}
