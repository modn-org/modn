#[path = "support/catalog.rs"]
mod support;
#[path = "support/tree.rs"]
mod tree;

use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::LazyLock;
use support::{error, Fixture};
use tree::{file_count, sha, snapshot};
use zip::write::SimpleFileOptions;

const CATALOG: &str = "../../buckets/main/games/cyberpunk-2077";

struct Release {
    id: String,
    version: String,
    dependencies: Vec<String>,
    versions: Vec<(String, String)>,
}

static STACK: LazyLock<Vec<Release>> = LazyLock::new(official_releases);

fn catalog_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CATALOG)
}

fn text(value: &Value) -> String {
    value.as_str().expect("string").to_owned()
}

fn official_releases() -> Vec<Release> {
    let mut releases: Vec<Release> = fs::read_dir(catalog_dir())
        .expect("official catalog")
        .map(|entry| {
            let path = entry.expect("entry").path();
            let manifest: Value =
                serde_json::from_slice(&fs::read(path).expect("manifest")).expect("JSON");
            Release {
                id: text(&manifest["id"]),
                version: text(&manifest["version"]),
                dependencies: manifest["dependencies"]
                    .as_array()
                    .expect("dependencies")
                    .iter()
                    .map(text)
                    .collect(),
                versions: manifest["dependency_versions"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(dependency, requirement)| (dependency.clone(), text(requirement)))
                    .collect(),
            }
        })
        .collect();
    releases.sort_by(|a, b| a.id.cmp(&b.id));
    releases
}

fn tops() -> Vec<&'static str> {
    STACK
        .iter()
        .filter(|release| {
            !STACK
                .iter()
                .any(|other| other.dependencies.contains(&release.id))
        })
        .map(|release| release.id.as_str())
        .collect()
}

struct Gate {
    fixture: Fixture,
    game: PathBuf,
    cache: PathBuf,
    archives: BTreeMap<String, PathBuf>,
    prefix: &'static str,
}

impl Gate {
    fn modn(&self, args: &[&str], files: &[&Path]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_modn"));
        command
            .arg("--state-dir")
            .arg(&self.fixture.state)
            .env("MODN_GAME_MODULES_DIR", &self.fixture.modules)
            .env("MODN_CACHE", &self.cache)
            .args(args);
        for file in files {
            command.arg("--file").arg(file);
        }
        command.output().expect("run CLI")
    }

    fn files_for(&self, top: &str) -> Vec<&Path> {
        let mut needed = vec![top];
        let mut index = 0;
        while index < needed.len() {
            let release = STACK
                .iter()
                .find(|release| release.id == needed[index])
                .expect("known mod");
            for dependency in &release.dependencies {
                if !needed.contains(&dependency.as_str()) {
                    needed.push(dependency);
                }
            }
            index += 1;
        }
        needed
            .iter()
            .filter_map(|id| self.archives.get(*id).map(PathBuf::as_path))
            .collect()
    }

    fn install(&self, top: &str, extra: &[&str]) -> Result<Vec<String>, String> {
        let target = format!("{}{top}", self.prefix);
        let mut args = vec!["--json", "install", target.as_str(), "--yes"];
        args.extend_from_slice(extra);
        let output = self.modn(&args, &self.files_for(top));
        if !output.status.success() {
            return Err(error(&output));
        }
        let report: Value = serde_json::from_slice(&output.stdout).expect("JSON");
        let ids = match report["stack"].as_array() {
            Some(stack) => stack
                .iter()
                .map(|member| member["installed"]["id"].as_str().expect("id").to_owned())
                .collect(),
            None => vec![report["installed"]["id"].as_str().expect("id").to_owned()],
        };
        Ok(ids)
    }

    fn managed(&self) -> Vec<Value> {
        let output = self.modn(&["--json", "list"], &[]);
        assert!(output.status.success(), "{}", error(&output));
        let list: Value = serde_json::from_slice(&output.stdout).expect("JSON");
        list.as_array().expect("array").clone()
    }

    fn uninstall(&self, id: &str) {
        let game = self.game.to_str().expect("UTF-8 game path");
        let output = self.modn(&["uninstall", id, "--yes", "--path", game], &[]);
        assert!(output.status.success(), "{id}: {}", error(&output));
    }

    fn run(&self, planted: &str, extra: &[&str]) -> usize {
        let before = snapshot(&self.game);

        let occupied = self.game.join(planted);
        fs::create_dir_all(occupied.parent().expect("parent")).expect("planted dir");
        fs::write(&occupied, b"unmanaged user file").expect("planted file");
        let expected = snapshot(&self.game);
        assert_eq!(file_count(&expected), file_count(&before) + 1);
        let message = self
            .install("virtual-atelier", extra)
            .expect_err("occupied destination must fail");
        assert!(message.contains("rolled back 5 package(s)"), "{message}");
        assert!(self.managed().is_empty());
        assert_eq!(snapshot(&self.game), expected);
        fs::remove_file(&occupied).expect("remove planted file");
        let mut parent = Path::new(planted).parent();
        while let Some(directory) = parent.filter(|p| !p.as_os_str().is_empty()) {
            let key = format!("{}/", directory.to_string_lossy().replace('\\', "/"));
            if !before.contains_key(&key) {
                fs::remove_dir(self.game.join(directory)).expect("remove planted parent");
            }
            parent = directory.parent();
        }
        assert_eq!(snapshot(&self.game), before);

        let ids = self.install("codeware", extra).expect("codeware stack");
        self.interrupt(&ids);
        let blocked = self
            .install("no-intro-videos", extra)
            .expect_err("pending stack");
        assert!(
            blocked.contains("unfinished installation stack"),
            "{blocked}"
        );
        let recovered = self.modn(&["--json", "recover", "--yes"], &[]);
        assert!(recovered.status.success(), "{}", error(&recovered));
        assert!(self.managed().is_empty());
        assert_eq!(snapshot(&self.game), before);

        let mut order = Vec::new();
        for top in tops() {
            order.extend(
                self.install(top, extra)
                    .unwrap_or_else(|e| panic!("{top}: {e}")),
            );
        }
        let managed = self.managed();
        assert_eq!(managed.len(), STACK.len());
        assert_eq!(order.len(), STACK.len());
        let owned: usize = managed
            .iter()
            .map(|package| package["files"].as_array().expect("files").len())
            .sum();
        assert_eq!(
            file_count(&snapshot(&self.game)),
            file_count(&before) + owned
        );
        for id in order.iter().rev() {
            self.uninstall(id);
        }
        assert!(self.managed().is_empty());
        assert_eq!(snapshot(&self.game), before);
        owned
    }

    fn interrupt(&self, ids: &[String]) {
        let connection = Connection::open(self.fixture.state.join("state.sqlite")).expect("db");
        let game: String = connection
            .query_row(
                "SELECT game_path FROM managed_packages WHERE id=?1",
                [&ids[0]],
                |row| row.get(0),
            )
            .expect("game path");
        connection
            .execute(
                "INSERT INTO pending_stacks(id, game_path) VALUES ('stack-gate', ?1)",
                [&game],
            )
            .expect("stack");
        for (ord, id) in ids.iter().enumerate() {
            connection
                .execute(
                    "INSERT INTO pending_stack_packages(stack_id, ord, package_id)
                     VALUES ('stack-gate', ?1, ?2)",
                    rusqlite::params![ord as i64, id],
                )
                .expect("member");
        }
    }
}

fn synthetic_gate() -> Gate {
    let fixture = Fixture::new();
    let bucket = fixture.temp.path().join("gate");
    let manifests = bucket.join("games/test-game");
    fs::create_dir_all(&manifests).expect("bucket dirs");
    fs::write(
        bucket.join("bucket.json"),
        r#"{"schema_version":1,"name":"gate","title":"Milestone gate"}"#,
    )
    .expect("bucket metadata");
    let mut archives = BTreeMap::new();
    for Release {
        id,
        version,
        dependencies,
        versions,
    } in STACK.iter()
    {
        let path = fixture.temp.path().join(format!("{id}.zip"));
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).expect("archive"));
        zip.start_file(
            format!("{id}/package/plugins/{id}.plugin"),
            SimpleFileOptions::default(),
        )
        .expect("entry");
        zip.write_all(format!("{id} {version}").as_bytes())
            .expect("payload");
        zip.finish().expect("finish");
        let requirements: serde_json::Map<String, Value> = versions
            .iter()
            .map(|(dependency, requirement)| (dependency.clone(), json!(requirement)))
            .collect();
        let manifest = json!({
            "schema_version":1, "id":id, "game":"test-game", "name":id,
            "version":version, "archive_format":"zip", "sha256":sha(&path),
            "source":{"kind":"manual","filename":format!("{id}.zip")},
            "dependencies":dependencies, "dependency_versions":requirements
        });
        fs::write(
            manifests.join(format!("{id}.json")),
            serde_json::to_vec(&manifest).expect("JSON"),
        )
        .expect("manifest");
        archives.insert(id.to_owned(), path);
    }
    let added = fixture.run_paths(&["bucket", "add", "gate"], &[&bucket]);
    assert!(added.status.success(), "{}", error(&added));
    fixture.register();
    Gate {
        game: fixture.game.clone(),
        cache: fixture.temp.path().join("cache"),
        fixture,
        archives,
        prefix: "gate/test-game/",
    }
}

#[test]
fn synthetic_full_stack_installs_and_rolls_back_reproducibly() {
    let gate = synthetic_gate();
    let owned = gate.run("addons/virtual-atelier.plugin", &[]);
    assert_eq!(owned, STACK.len());
}

#[test]
#[ignore = "needs MODN_GATE_ARCHIVES with the Nexus archives, and network access"]
fn real_cyberpunk_stack_installs_and_rolls_back_in_a_disposable_directory() {
    let source = PathBuf::from(
        std::env::var_os("MODN_GATE_ARCHIVES")
            .expect("set MODN_GATE_ARCHIVES to the archive folder"),
    );
    let fixture = Fixture::new();
    let game = fixture.temp.path().join("cyberpunk");
    fs::create_dir_all(game.join("bin/x64")).expect("game");
    fs::write(
        game.join("bin/x64/Cyberpunk2077.exe"),
        b"disposable fixture",
    )
    .expect("marker");

    let catalog = catalog_dir();
    let mut digests = BTreeMap::new();
    for entry in fs::read_dir(&source).expect("archive folder") {
        let path = entry.expect("entry").path();
        if path.is_file() {
            digests.insert(sha(&path), path);
        }
    }
    let mut archives = BTreeMap::new();
    for Release { id, .. } in STACK.iter() {
        let manifest: Value = serde_json::from_slice(
            &fs::read(catalog.join(format!("{id}.json"))).expect("manifest"),
        )
        .expect("JSON");
        if manifest["source"]["kind"] == "manual" {
            let sha256 = manifest["sha256"].as_str().expect("sha256");
            let path = digests.get(sha256).unwrap_or_else(|| {
                panic!(
                    "{id}: no archive with SHA-256 {sha256} in {}",
                    source.display()
                )
            });
            archives.insert(id.to_owned(), path.clone());
        }
    }

    let cache = std::env::var_os("MODN_GATE_CACHE")
        .map_or_else(|| fixture.temp.path().join("cache"), PathBuf::from);
    let gate = Gate {
        game,
        cache,
        fixture,
        archives,
        prefix: "main/cyberpunk-2077/",
    };
    let registered = gate.modn(
        &[
            "game",
            "add",
            "cyberpunk-2077",
            "--path",
            gate.game.to_str().expect("UTF-8"),
        ],
        &[],
    );
    assert!(registered.status.success(), "{}", error(&registered));
    let owned = gate.run(
        "archive/pc/mod/VirtualAtelier.archive",
        &["--allow-executable", "engine/tools/scc.exe"],
    );
    assert_eq!(owned, 147, "owned files of the official stack");
}
