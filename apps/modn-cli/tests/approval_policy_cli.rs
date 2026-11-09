#[path = "support/main_bucket.rs"]
mod main_bucket;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use tempfile::{tempdir, TempDir};
use zip::write::SimpleFileOptions;

const GAME: &str = "tools-game";
const MOD: &str = "tool-mod";
const FIRST: &str = "tools/one.exe";
const SECOND: &str = "tools/two.exe";

const MODULE_LUA: &str = r#"
local policy = {
    game_id = "tools-game",
    api_version = mods_host.api_version,
    marker = "bin/TestGame.exe",
    steam_manifest = "appmanifest_000000.acf",
    common_paths = {},
}
function policy.validate()
    local ok, info = pcall(mods_host.fs.metadata, 1, policy.marker)
    return ok and info.kind == "file"
end
function policy.steam_name() return "Tools Game" end
function policy.is_epic(name) return name == "Tools Game" end
function policy.scan(limit)
    return { mods = {}, framework_signals = {}, incomplete = false, redmod_available = false }
end
function policy.doctor(report) return {} end
function policy.plan(files)
    local operations = {}
    local blockers = {}
    for _, file in ipairs(files) do
        if string.match(file.source, "^tools/[^/]+%.exe$") then
            table.insert(operations, {
                source = file.source, destination = file.source,
                kind = "tool", package_root = "",
            })
        else
            table.insert(blockers, { code = "UNMAPPED", message = "Unmapped: " .. file.source })
        end
    end
    return { schema_version = 1, operations = operations, blockers = blockers }
end
return policy
"#;

struct Fixture {
    temp: TempDir,
    game: PathBuf,
    state: PathBuf,
    modules: PathBuf,
}

fn copy_module(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("module target");
    for entry in fs::read_dir(source).expect("module source") {
        let entry = entry.expect("module entry");
        if entry.file_type().expect("module entry type").is_file() {
            fs::copy(entry.path(), target.join(entry.file_name())).expect("module copy");
        }
    }
}

impl Fixture {
    fn new() -> Self {
        let temp = tempdir().expect("fixture");
        let game = temp.path().join("game");
        fs::create_dir_all(game.join("bin")).expect("game directory");
        fs::write(game.join("bin/TestGame.exe"), b"game").expect("game marker");
        let state = temp.path().join("state");
        let modules = temp.path().join("modules");
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for (source, name) in [
            ("buckets/main/games/cyberpunk-2077", "cyberpunk-2077"),
            ("buckets/main/games/balatro", "balatro"),
            ("tests/fixtures/test-game", "test-game"),
            (
                "apps/modn-cli/tests/fixtures/native-injector-game",
                "native-injector-game",
            ),
        ] {
            copy_module(&repo.join(source), &modules.join(name));
        }
        let module = modules.join(GAME);
        fs::create_dir_all(&module).expect("module directory");
        fs::write(
            module.join("module.json"),
            format!(
                r#"{{"game_id":"{GAME}","version":"0.1.0","api_version":1,"entrypoint":"main.lua"}}"#
            ),
        )
        .expect("module metadata");
        fs::write(module.join("main.lua"), MODULE_LUA).expect("module source");
        Self {
            temp,
            game,
            state,
            modules,
        }
    }

    fn command(&self) -> main_bucket::SeededCommand {
        let mut command = main_bucket::command(env!("CARGO_BIN_EXE_modn"));
        command
            .args(["--game", GAME, "--json", "--state-dir"])
            .arg(&self.state)
            .env("MODN_GAME_MODULES_DIR", &self.modules);
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().expect("run CLI")
    }

    fn run_env(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut command = self.command();
        command.args(args);
        for (key, value) in env {
            command.env(key, value);
        }
        command.output().expect("run CLI")
    }

    fn zip(&self, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let path = self.temp.path().join(name);
        let mut writer = zip::ZipWriter::new(File::create(&path).expect("ZIP"));
        for &(member, content) in files {
            writer
                .start_file(member, SimpleFileOptions::default())
                .expect("ZIP member");
            writer.write_all(content).expect("ZIP content");
        }
        writer.finish().expect("ZIP complete");
        path
    }

    fn write_bucket(&self, version: &str, archive: &Path, executables: &[&str]) {
        let root = self.temp.path().join("tools");
        let games = root.join("games").join(GAME);
        fs::create_dir_all(&games).expect("bucket directories");
        fs::write(
            root.join("bucket.json"),
            r#"{"schema_version":1,"name":"tools","title":"Tools"}"#,
        )
        .expect("bucket metadata");
        let sha256 = format!("{:x}", Sha256::digest(fs::read(archive).expect("archive")));
        let manifest = json!({
            "schema_version": 1, "id": MOD, "game": GAME, "name": "Tool Mod",
            "version": version, "archive_format": "zip", "sha256": sha256,
            "source": {"kind": "manual", "filename": "tool.zip"},
            "executables": executables,
        });
        fs::write(
            games.join(format!("{MOD}.json")),
            serde_json::to_vec(&manifest).expect("manifest JSON"),
        )
        .expect("manifest");
    }

    fn register_bucket(&self) {
        let root = self.temp.path().join("tools");
        let output = self.run(&["bucket", "add", "tools", root.to_str().expect("bucket")]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn install(&self, archive: &Path, extra: &[&str]) -> Output {
        let mut args = vec![
            "install",
            MOD,
            "--path",
            self.game.to_str().expect("game"),
            "--file",
            archive.to_str().expect("archive"),
        ];
        args.extend_from_slice(extra);
        self.run(&args)
    }

    fn update(&self, archive: &Path, extra: &[&str]) -> Output {
        let mut args = vec![
            "update",
            MOD,
            "--path",
            self.game.to_str().expect("game"),
            "--file",
            archive.to_str().expect("archive"),
        ];
        args.extend_from_slice(extra);
        self.run(&args)
    }

    fn stored_paths(&self) -> Option<Vec<String>> {
        if !self.state.join("state.sqlite").is_file() {
            return None;
        }
        let database = rusqlite::Connection::open(self.state.join("state.sqlite")).ok()?;
        let stored: Option<String> = database
            .query_row(
                "SELECT executables FROM mod_approvals WHERE mod_id=?1",
                [MOD],
                |row| row.get(0),
            )
            .ok();
        stored.map(|paths| serde_json::from_str(&paths).expect("executable JSON"))
    }
}

fn refusal(output: &Output) -> Value {
    serde_json::from_slice(&output.stderr).expect("structured refusal")
}

#[test]
fn executable_approval_is_stored_and_covers_a_reinstall() {
    let fixture = Fixture::new();
    let v1 = fixture.zip("v1.zip", &[(FIRST, b"one")]);
    fixture.write_bucket("1.0", &v1, &[FIRST]);
    fixture.register_bucket();

    let preview = fixture.install(&v1, &["--dry-run"]);
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    let preview: Value = serde_json::from_slice(&preview.stdout).expect("preview JSON");
    assert_eq!(preview["approval"]["required"], true, "{preview}");
    assert_eq!(preview["approval"]["covered_by_stored_approval"], false);
    assert_eq!(
        preview["approval"]["uncovered"]["executables"],
        json!([FIRST])
    );
    assert_eq!(fixture.stored_paths(), None, "a dry run stores nothing");

    let refused = fixture.install(&v1, &["--yes"]);
    assert!(!refused.status.success());
    let refusal = refusal(&refused);
    assert_eq!(refusal["error"], "APPROVAL_REQUIRED", "{refusal}");
    assert_eq!(refusal["mod_id"], MOD);
    assert_eq!(refusal["uncovered"]["executables"], json!([FIRST]));
    assert!(!fixture.game.join(FIRST).exists());
    assert_eq!(fixture.stored_paths(), None, "a refusal stores nothing");

    let installed = fixture.install(&v1, &["--yes", "--allow-executable", FIRST]);
    assert!(
        installed.status.success(),
        "{}",
        String::from_utf8_lossy(&installed.stderr)
    );
    assert_eq!(fs::read(fixture.game.join(FIRST)).expect("payload"), b"one");
    assert_eq!(fixture.stored_paths(), Some(vec![FIRST.to_owned()]));

    let uninstalled = fixture.run(&[
        "uninstall",
        MOD,
        "--path",
        fixture.game.to_str().expect("game"),
        "--yes",
    ]);
    assert!(
        uninstalled.status.success(),
        "{}",
        String::from_utf8_lossy(&uninstalled.stderr)
    );
    let covered = fixture.install(&v1, &["--dry-run"]);
    let covered: Value = serde_json::from_slice(&covered.stdout).expect("preview JSON");
    assert_eq!(covered["approval"]["required"], false, "{covered}");
    assert_eq!(covered["approval"]["covered_by_stored_approval"], true);

    let reinstalled = fixture.install(&v1, &["--yes"]);
    assert!(
        reinstalled.status.success(),
        "{}",
        String::from_utf8_lossy(&reinstalled.stderr)
    );
    assert!(fixture.game.join(FIRST).is_file());
}

#[test]
fn a_new_executable_path_is_refused_and_the_flag_widens_the_approval() {
    let fixture = Fixture::new();
    let v1 = fixture.zip("v1.zip", &[(FIRST, b"one")]);
    let v2 = fixture.zip("v2.zip", &[(FIRST, b"one"), (SECOND, b"two")]);
    fixture.write_bucket("1.0", &v1, &[FIRST]);
    fixture.register_bucket();
    let installed = fixture.install(&v1, &["--yes", "--allow-executable", FIRST]);
    assert!(
        installed.status.success(),
        "{}",
        String::from_utf8_lossy(&installed.stderr)
    );

    fixture.write_bucket("2.0", &v2, &[FIRST, SECOND]);
    let refused = fixture.update(&v2, &["--yes"]);
    assert!(!refused.status.success());
    let refusal = refusal(&refused);
    assert_eq!(refusal["error"], "APPROVAL_REQUIRED", "{refusal}");
    assert_eq!(refusal["uncovered"]["executables"], json!([SECOND]));
    assert!(!fixture.game.join(SECOND).exists());
    assert_eq!(fixture.stored_paths(), Some(vec![FIRST.to_owned()]));

    let widened = fixture.update(&v2, &["--yes", "--allow-executable", SECOND]);
    assert!(
        widened.status.success(),
        "{}",
        String::from_utf8_lossy(&widened.stderr)
    );
    assert!(fixture.game.join(SECOND).is_file());
    assert_eq!(
        fixture.stored_paths(),
        Some(vec![FIRST.to_owned(), SECOND.to_owned()])
    );

    let uninstalled = fixture.run(&[
        "uninstall",
        MOD,
        "--path",
        fixture.game.to_str().expect("game"),
        "--yes",
    ]);
    assert!(
        uninstalled.status.success(),
        "{}",
        String::from_utf8_lossy(&uninstalled.stderr)
    );
    let reinstalled = fixture.install(&v2, &["--yes"]);
    assert!(
        reinstalled.status.success(),
        "{}",
        String::from_utf8_lossy(&reinstalled.stderr)
    );
}

#[test]
fn a_rolled_back_update_does_not_widen_the_approval() {
    let fixture = Fixture::new();
    let v1 = fixture.zip("v1.zip", &[(FIRST, b"one")]);
    let v2 = fixture.zip("v2.zip", &[(FIRST, b"one"), (SECOND, b"two")]);
    fixture.write_bucket("1.0", &v1, &[FIRST]);
    fixture.register_bucket();
    let installed = fixture.install(&v1, &["--yes", "--allow-executable", FIRST]);
    assert!(
        installed.status.success(),
        "{}",
        String::from_utf8_lossy(&installed.stderr)
    );

    fixture.write_bucket("2.0", &v2, &[FIRST, SECOND]);
    let failed = fixture.run_env(
        &[
            "update",
            MOD,
            "--path",
            fixture.game.to_str().expect("game"),
            "--file",
            v2.to_str().expect("archive"),
            "--yes",
            "--allow-executable",
            SECOND,
        ],
        &[("MODN_TEST_UPDATE_FAULT_AFTER", "1")],
    );
    assert!(!failed.status.success(), "the injected fault must fail");
    assert_eq!(
        fixture.stored_paths(),
        Some(vec![FIRST.to_owned()]),
        "a rolled-back update must not widen approval"
    );

    let widened = fixture.update(&v2, &["--yes", "--allow-executable", SECOND]);
    assert!(
        widened.status.success(),
        "{}",
        String::from_utf8_lossy(&widened.stderr)
    );
    assert_eq!(
        fixture.stored_paths(),
        Some(vec![FIRST.to_owned(), SECOND.to_owned()])
    );
}

#[test]
fn a_cancelled_install_stores_no_approval() {
    let fixture = Fixture::new();
    let v1 = fixture.zip("v1.zip", &[(FIRST, b"one")]);
    fixture.write_bucket("1.0", &v1, &[FIRST]);
    fixture.register_bucket();

    let mut command = fixture.command();
    let mut child = command
        .args([
            "--progress",
            "json",
            "install",
            MOD,
            "--path",
            fixture.game.to_str().expect("game"),
            "--file",
            v1.to_str().expect("archive"),
            "--yes",
            "--allow-executable",
            FIRST,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn modn");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"cancel\n")
        .expect("cancel line");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("modn output");
    assert!(!output.status.success());
    assert_eq!(fixture.stored_paths(), None);
    assert!(!fixture.game.join(FIRST).exists());
}
