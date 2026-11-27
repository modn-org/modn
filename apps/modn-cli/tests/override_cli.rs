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
