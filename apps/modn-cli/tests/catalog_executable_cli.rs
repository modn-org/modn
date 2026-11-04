#[path = "support/main_bucket.rs"]
mod main_bucket;
use serde_json::{json, Value};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Output;
use tempfile::{tempdir, TempDir};
use zip::write::SimpleFileOptions;

const COMPILER: &str = "engine/tools/scc.exe";
const REDSCRIPT: &str = "community/cyberpunk-2077/redscript";
const DEPENDENT: &str = "community/cyberpunk-2077/script-mod";
const SMUGGLER: &str = "community/cyberpunk-2077/smuggler";
const EXTRA_TOOL: &str = "engine/tools/extra.exe";

struct Catalog {
    fixture: Fixture,
    redscript: PathBuf,
    dependent: PathBuf,
    smuggler: PathBuf,
}

struct Fixture {
    temp: TempDir,
    game: PathBuf,
    state: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempdir().expect("fixture");
        let game = temp.path().join("game");
        fs::create_dir_all(game.join("bin/x64")).expect("game directories");
        fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").expect("game marker");
        let state = temp.path().join("state");
        Self { temp, game, state }
    }

    fn cli(&self, args: &[&str]) -> Output {
        main_bucket::command(env!("CARGO_BIN_EXE_modn"))
            .args(["--game", "cyberpunk-2077", "--json", "--state-dir"])
            .arg(&self.state)
            .args(args)
            .output()
            .expect("CLI")
    }

    fn json(&self, args: &[&str]) -> Value {
        let result = self.cli(args);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        serde_json::from_slice(&result.stdout).expect("CLI JSON")
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

    fn digest(&self, archive: &Path) -> String {
        let preview = self.json(&[
            "install",
            archive.to_str().expect("archive"),
            "--dry-run",
            "--path",
            self.game.to_str().expect("game"),
        ]);
        preview["archive_sha256"]
            .as_str()
            .expect("digest")
            .to_owned()
    }

    fn register_bucket(&self, packages: &[Value]) {
        let root = self.temp.path().join("community");
        let games = root.join("games/cyberpunk-2077");
        fs::create_dir_all(&games).expect("bucket directories");
        fs::write(
            root.join("bucket.json"),
            r#"{"schema_version":1,"name":"community","title":"Community"}"#,
        )
        .expect("bucket metadata");
        for package in packages {
            let id = package["id"].as_str().expect("package ID");
            fs::write(
                games.join(format!("{id}.json")),
                serde_json::to_vec(package).expect("manifest JSON"),
            )
            .expect("manifest");
        }
        self.json(&["bucket", "add", "community", root.to_str().expect("bucket")]);
    }

    fn install(&self, reference: &str, archive: &Path, extra: &[&str]) -> Output {
        let mut args = vec![
            "install",
            reference,
            "--file",
            archive.to_str().expect("archive"),
            "--path",
            self.game.to_str().expect("game"),
        ];
        args.extend_from_slice(extra);
        self.cli(&args)
    }
}

fn manifest(id: &str, sha256: &str, dependencies: &[&str]) -> Value {
    json!({
        "schema_version": 1, "id": id, "game": "cyberpunk-2077", "name": id,
        "version": "1.0", "archive_format": "zip", "sha256": sha256,
        "source": {"kind": "manual", "filename": format!("{id}.zip")},
        "dependencies": dependencies,
    })
}

fn declaring(mut manifest: Value, executables: &[&str]) -> Value {
    manifest["executables"] = json!(executables);
    manifest
}

fn catalog() -> Catalog {
    let fixture = Fixture::new();
    let compiler_files: [(&str, &[u8]); 4] = [
        (COMPILER, b"compiler"),
        ("engine/tools/scc_lib.dll", b"library"),
        ("engine/config/base/scripts.ini", b"ini"),
        ("r6/config/cybercmd/scc.toml", b"config"),
    ];
    let redscript = fixture.zip("redscript.zip", &compiler_files);
    let mut smuggled = compiler_files.to_vec();
    smuggled.push((EXTRA_TOOL, b"extra"));
    let smuggler = fixture.zip("smuggler.zip", &smuggled);
    let dependent = fixture.zip("script-mod.zip", &[("r6/scripts/Mod/Mod.reds", b"script")]);
    let packages = [
        declaring(
            manifest("redscript", &fixture.digest(&redscript), &[]),
            &[COMPILER],
        ),
        manifest("script-mod", &fixture.digest(&dependent), &["redscript"]),
        declaring(
            manifest("smuggler", &fixture.digest(&smuggler), &[]),
            &[COMPILER],
        ),
    ];
    fixture.register_bucket(&packages);
    fixture.json(&[
        "game",
        "add",
        "cyberpunk-2077",
        "--path",
        fixture.game.to_str().expect("game"),
    ]);
    Catalog {
        fixture,
        redscript,
        dependent,
        smuggler,
    }
}

fn blockers(plan: &Value) -> Vec<&str> {
    plan["plan"]["blockers"]
        .as_array()
        .expect("blockers")
        .iter()
        .map(|blocker| blocker["code"].as_str().expect("code"))
        .collect()
}

#[test]
fn catalog_compiler_requires_exact_operator_approval() {
    let Catalog {
        fixture,
        redscript,
        smuggler,
        ..
    } = catalog();
    let denied = fixture.install(REDSCRIPT, &redscript, &["--dry-run"]);
    let plan: Value = serde_json::from_slice(&denied.stdout).expect("plan JSON");
    assert_eq!(plan["plan"]["status"], "blocked", "{plan}");
    assert!(blockers(&plan).contains(&"EXECUTABLE_PAYLOAD"), "{plan}");

    let wrong = ["--allow-executable", "engine/tools/other.exe"];
    for extra in [&["--yes"][..], &[&["--yes"][..], &wrong[..]].concat()] {
        let result = fixture.install(REDSCRIPT, &redscript, extra);
        assert!(!result.status.success(), "{extra:?}");
        assert!(!fixture.game.join(COMPILER).exists(), "{extra:?}");
    }

    let approval = ["--allow-executable", COMPILER];
    let smuggled = fixture.install(
        SMUGGLER,
        &smuggler,
        &[&approval[..], &["--dry-run"]].concat(),
    );
    let plan: Value = serde_json::from_slice(&smuggled.stdout).expect("plan JSON");
    assert_eq!(plan["plan"]["status"], "blocked", "{plan}");
    assert!(blockers(&plan).contains(&"EXECUTABLE_PAYLOAD"), "{plan}");
    let smuggled = fixture.install(SMUGGLER, &smuggler, &[&approval[..], &["--yes"]].concat());
    assert!(!smuggled.status.success());
    assert!(!fixture.game.join(COMPILER).exists());
    assert!(!fixture.game.join(EXTRA_TOOL).exists());
}

#[test]
fn approved_catalog_compiler_unblocks_dependents_and_uninstalls_cleanly() {
    let Catalog {
        fixture,
        redscript,
        dependent,
        ..
    } = catalog();
    let approval = ["--allow-executable", COMPILER];
    let plan = fixture.install(
        REDSCRIPT,
        &redscript,
        &[&approval[..], &["--dry-run"]].concat(),
    );
    let plan: Value = serde_json::from_slice(&plan.stdout).expect("plan JSON");
    assert_eq!(plan["plan"]["status"], "reviewable", "{plan}");

    let unapproved = fixture.install(REDSCRIPT, &redscript, &["--yes"]);
    assert!(!unapproved.status.success());
    assert!(!fixture.game.join(COMPILER).exists());

    let installed = fixture.install(REDSCRIPT, &redscript, &[&approval[..], &["--yes"]].concat());
    assert!(
        installed.status.success(),
        "{}",
        String::from_utf8_lossy(&installed.stderr)
    );
    assert_eq!(
        fs::read(fixture.game.join(COMPILER)).expect("compiler"),
        b"compiler"
    );

    let script = fixture.install(DEPENDENT, &dependent, &["--yes"]);
    assert!(
        script.status.success(),
        "{}",
        String::from_utf8_lossy(&script.stderr)
    );
    let script: Value = serde_json::from_slice(&script.stdout).expect("install JSON");
    let script_id = script["installed"]["id"].as_str().expect("package ID");
    let redscript_id = serde_json::from_slice::<Value>(&installed.stdout).expect("install JSON")
        ["installed"]["id"]
        .as_str()
        .expect("package ID")
        .to_owned();

    let blocked = fixture.cli(&["uninstall", &redscript_id, "--yes"]);
    assert!(!blocked.status.success(), "dependent must block uninstall");
    for id in [script_id, &redscript_id] {
        let removed = fixture.cli(&["uninstall", id, "--yes"]);
        assert!(
            removed.status.success(),
            "{}",
            String::from_utf8_lossy(&removed.stderr)
        );
    }
    assert!(!fixture.game.join(COMPILER).exists());
    assert!(!fixture.game.join("r6/scripts/Mod/Mod.reds").exists());
}

#[test]
fn declared_executable_is_reported_but_never_approved_by_yes() {
    let Catalog {
        fixture, redscript, ..
    } = catalog();
    let preview = fixture.install(REDSCRIPT, &redscript, &["--dry-run"]);
    let preview: Value = serde_json::from_slice(&preview.stdout).expect("plan JSON");
    assert_eq!(preview["plan"]["status"], "blocked", "{preview}");
    assert_eq!(
        preview["unapproved_declared_executables"],
        json!([COMPILER])
    );

    let approval = ["--allow-executable", COMPILER];
    let approved = fixture.install(
        REDSCRIPT,
        &redscript,
        &[&approval[..], &["--dry-run"]].concat(),
    );
    let approved: Value = serde_json::from_slice(&approved.stdout).expect("plan JSON");
    assert_eq!(approved["plan"]["status"], "reviewable", "{approved}");
    assert_eq!(approved["unapproved_declared_executables"], json!([]));

    let refused = fixture.install(REDSCRIPT, &redscript, &["--yes"]);
    assert!(!refused.status.success());
    let message = String::from_utf8_lossy(&refused.stderr);
    assert!(
        message.contains(COMPILER) && message.contains("--allow-executable"),
        "{message}"
    );
    assert!(!fixture.game.join(COMPILER).exists());

    let unattended = fixture.install(REDSCRIPT, &redscript, &[]);
    assert!(!unattended.status.success());
    assert!(String::from_utf8_lossy(&unattended.stderr).contains("confirm in a terminal"));
    assert!(!fixture.game.join(COMPILER).exists());
}

#[test]
fn manifests_cannot_declare_executables_outside_tool_directories() {
    for executables in [
        &["bin/x64/Cyberpunk2077.exe"][..],
        &["engine/tools/scc.dll"],
        &["engine/tools/../scc.exe"],
        &[COMPILER, COMPILER],
    ] {
        let fixture = Fixture::new();
        let root = fixture.temp.path().join("community");
        let games = root.join("games/cyberpunk-2077");
        fs::create_dir_all(&games).expect("bucket directories");
        fs::write(
            root.join("bucket.json"),
            r#"{"schema_version":1,"name":"community","title":"Community"}"#,
        )
        .expect("bucket metadata");
        let package = declaring(manifest("tool", &"0".repeat(64), &[]), executables);
        fs::write(
            games.join("tool.json"),
            serde_json::to_vec(&package).expect("JSON"),
        )
        .expect("manifest");
        let added = fixture.cli(&["bucket", "add", "community", root.to_str().expect("bucket")]);
        assert!(!added.status.success(), "{executables:?}");
    }
}
