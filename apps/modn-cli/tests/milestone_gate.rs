//! Roadmap milestone 3 exit gate: reproducible disposable full-stack
//! installation and rollback.
//!
//! The synthetic gate runs offline in CI. It mirrors the official Cyberpunk
//! prerequisite graph and version requirements with the test-game policy.
//!
//! The real gate installs the official eleven-mod stack into a disposable
//! fake Cyberpunk directory, never a real game. It downloads the `https`
//! archives and needs the four Nexus archives locally:
//!
//! ```text
//! MODN_GATE_ARCHIVES=~/Downloads/CP77 cargo test -p modn-cli --test milestone_gate -- --ignored
//! ```
//!
//! `MODN_GATE_CACHE` optionally reuses a download cache between runs.
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
use support::{error, Fixture};
use tree::{file_count, sha, snapshot};
use zip::write::SimpleFileOptions;

/// One official prerequisite release and its declared requirements.
struct Release {
    id: &'static str,
    version: &'static str,
    dependencies: &'static [&'static str],
    versions: &'static [(&'static str, &'static str)],
}

/// The official prerequisite graph, with its version requirements.
const STACK: [Release; 11] = [
    Release {
        id: "red4ext",
        version: "1.31.0",
        dependencies: &[],
        versions: &[],
    },
    Release {
        id: "redscript",
        version: "0.5.32",
        dependencies: &[],
        versions: &[],
    },
    Release {
        id: "cet",
        version: "1.38.1",
        dependencies: &["red4ext"],
        versions: &[],
    },
    Release {
        id: "archivexl",
        version: "1.28.6",
        dependencies: &["red4ext", "redscript"],
        versions: &[("red4ext", ">=1.31.0"), ("redscript", ">=0.5.32")],
    },
    Release {
        id: "tweakxl",
        version: "1.12.0",
        dependencies: &["red4ext", "redscript"],
        versions: &[("red4ext", ">=1.31.0"), ("redscript", ">=0.5.32")],
    },
    Release {
        id: "codeware",
        version: "1.21.2",
        dependencies: &["red4ext", "redscript"],
        versions: &[("red4ext", ">=1.31.0"), ("redscript", ">=0.5.32")],
    },
    Release {
        id: "mod-settings",
        version: "0.2.28",
        dependencies: &["archivexl", "red4ext", "redscript"],
        versions: &[],
    },
    Release {
        id: "equipment-ex",
        version: "1.2.10",
        dependencies: &["archivexl", "codeware", "redscript", "tweakxl"],
        versions: &[("archivexl", ">=1.28.0"), ("tweakxl", ">=1.12.0")],
    },
    Release {
        id: "virtual-atelier",
        version: "1.6.4",
        dependencies: &["archivexl", "codeware", "mod-settings", "redscript"],
        versions: &[],
    },
    Release {
        id: "browser-extension",
        version: "0.9.7",
        dependencies: &["codeware", "red4ext", "redscript"],
        versions: &[],
    },
    Release {
        id: "no-intro-videos",
        version: "0.8",
        dependencies: &["redscript"],
        versions: &[],
    },
];

/// Mods that pull in the whole stack, in the order the gate installs them.
const TOPS: [&str; 5] = [
    "virtual-atelier",
    "equipment-ex",
    "browser-extension",
    "no-intro-videos",
    "cet",
];

struct Gate {
    fixture: Fixture,
    game: PathBuf,
    cache: PathBuf,
    /// Supplied archive for each mod that needs one, by mod ID.
    archives: BTreeMap<String, PathBuf>,
    /// `bucket/game/` prefix that makes each mod reference unambiguous.
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

    /// Supplied archives for `top` and every mod it depends on.
    fn files_for(&self, top: &str) -> Vec<&Path> {
        let mut needed = vec![top];
        let mut index = 0;
        while index < needed.len() {
            let release = STACK
                .iter()
                .find(|release| release.id == needed[index])
                .expect("known mod");
            for dependency in release.dependencies {
                if !needed.contains(dependency) {
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

    /// Install `top` with its missing dependencies; return package IDs in
    /// install order.
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

    /// Run the gate: rollback, interrupted recovery, then full install and
    /// reverse uninstall. Returns the owned file count of the full stack.
    fn run(&self, planted: &str, extra: &[&str]) -> usize {
        let before = snapshot(&self.game);

        // 1. A failure in the last member rolls the whole stack back.
        let occupied = self.game.join(planted);
        fs::create_dir_all(occupied.parent().expect("parent")).expect("planted dir");
        fs::write(&occupied, b"unmanaged user file").expect("planted file");
        // Captured before the install, so a changed planted file fails.
        let expected = snapshot(&self.game);
        assert_eq!(file_count(&expected), file_count(&before) + 1);
        let message = self
            .install("virtual-atelier", extra)
            .expect_err("occupied destination must fail");
        assert!(message.contains("rolled back 5 package(s)"), "{message}");
        assert!(self.managed().is_empty());
        assert_eq!(snapshot(&self.game), expected);
        fs::remove_file(&occupied).expect("remove planted file");
        // The test, not Modn, created the planted file's missing parents.
        let mut parent = Path::new(planted).parent();
        while let Some(directory) = parent.filter(|p| !p.as_os_str().is_empty()) {
            let key = format!("{}/", directory.to_string_lossy().replace('\\', "/"));
            if !before.contains_key(&key) {
                fs::remove_dir(self.game.join(directory)).expect("remove planted parent");
            }
            parent = directory.parent();
        }
        assert_eq!(snapshot(&self.game), before);

        // 2. A stack interrupted before it commits is rolled back by recovery.
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

        // 3. The full stack installs, then uninstalls in reverse order.
        let mut order = Vec::new();
        for top in TOPS {
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

    /// Recreate the state a crash leaves after `ids` committed as members of
    /// a stack that never finished.
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
    } in STACK
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
            .map(|(dependency, requirement)| ((*dependency).to_owned(), json!(requirement)))
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
    // The synthetic graph must stay the official one.
    let catalog =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main/games/cyberpunk-2077");
    let mut official: Vec<String> = fs::read_dir(&catalog)
        .expect("official catalog")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    official.sort();
    let mut synthetic: Vec<String> = STACK.iter().map(|r| format!("{}.json", r.id)).collect();
    synthetic.sort();
    assert_eq!(synthetic, official, "same mods as buckets/main");
    for release in &STACK {
        let manifest: Value = serde_json::from_slice(
            &fs::read(catalog.join(format!("{}.json", release.id))).expect("manifest"),
        )
        .expect("JSON");
        let requirements: serde_json::Map<String, Value> = release
            .versions
            .iter()
            .map(|(id, requirement)| ((*id).to_owned(), json!(requirement)))
            .collect();
        assert_eq!(manifest["version"], release.version, "{}", release.id);
        assert_eq!(
            manifest["dependencies"],
            json!(release.dependencies),
            "{}",
            release.id
        );
        assert_eq!(
            manifest
                .get("dependency_versions")
                .cloned()
                .unwrap_or_else(|| json!({})),
            Value::Object(requirements),
            "{}",
            release.id
        );
    }

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

    // Pick each manual archive by the digest its official manifest pins.
    let catalog =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main/games/cyberpunk-2077");
    let mut digests = BTreeMap::new();
    for entry in fs::read_dir(&source).expect("archive folder") {
        let path = entry.expect("entry").path();
        if path.is_file() {
            digests.insert(sha(&path), path);
        }
    }
    let mut archives = BTreeMap::new();
    for Release { id, .. } in STACK {
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
    assert_eq!(archives.len(), 4, "four Nexus archives");

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
    assert_eq!(owned, 137, "owned files of the official stack");
}
