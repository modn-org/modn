#[path = "support/main_bucket.rs"]
mod main_bucket;
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

const SIGNED_KEY: &str = "40C666AA387404CE";
const LOCAL_SKIP: &str = "local folder bucket has nothing to fetch";

fn pack(source: &Path, zip: &Path) {
    fn files(root: &Path, current: &Path, output: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(current).expect("fixture directory") {
            let path = entry.expect("fixture entry").path();
            if path.file_name().is_some_and(|name| name == ".git") {
                continue;
            }
            if path.is_dir() {
                files(root, &path, output);
            } else {
                output.push((
                    path.strip_prefix(root)
                        .expect("fixture relative path")
                        .to_str()
                        .expect("fixture UTF-8 path")
                        .replace('\\', "/"),
                    fs::read(path).expect("fixture file"),
                ));
            }
        }
    }
    let mut entries = Vec::new();
    files(source, source, &mut entries);
    entries.sort();
    let mut writer = zip::ZipWriter::new(fs::File::create(zip).expect("fixture ZIP"));
    for (name, bytes) in entries {
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .expect("fixture ZIP entry");
        writer.write_all(&bytes).expect("fixture ZIP bytes");
    }
    writer.finish().expect("fixture ZIP complete");
}

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("fixture directory");
    for entry in fs::read_dir(source).expect("fixture entries") {
        let entry = entry.expect("fixture entry");
        if entry.file_name() == ".git" {
            continue;
        }
        let target = target.join(entry.file_name());
        if entry.file_type().expect("fixture type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::write(&target, fs::read(entry.path()).expect("fixture bytes"))
                .expect("fixture file");
        }
    }
}

fn catalog_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/signed-catalog")
        .join(name)
}

fn replace_catalog(repo: &Path, name: &str) {
    for entry in fs::read_dir(repo).expect("repository entries") {
        let entry = entry.expect("repository entry");
        if entry.file_name() == ".git" {
            continue;
        }
        if entry.file_type().expect("repository type").is_dir() {
            fs::remove_dir_all(entry.path()).expect("remove old catalog directory");
        } else {
            fs::remove_file(entry.path()).expect("remove old catalog file");
        }
    }
    copy_tree(&catalog_fixture(name), repo);
}

fn git(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {arguments:?}: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git output")
        .trim()
        .to_owned()
}

fn commit_all(repo: &Path, message: &str) -> String {
    git(repo, &["add", "-A"]);
    git(
        repo,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            message,
        ],
    );
    git(repo, &["rev-parse", "HEAD"])
}

struct Fixture {
    _temp: TempDir,
    data: PathBuf,
    main_zip: PathBuf,
    repo: PathBuf,
    local: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().expect("test fixture invariant");
        let data = temp.path().join("data");
        fs::create_dir_all(&data).expect("data directory");
        let tree = temp.path().join("main-tree");
        copy_tree(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main"),
            &tree,
        );
        let main_zip = temp.path().join("main.zip");
        pack(&tree, &main_zip);
        main_bucket::write_signature(&tree, &main_zip);
        let repo = temp.path().join("signed-repo");
        fs::create_dir_all(&repo).expect("repository directory");
        git(&repo, &["init", "--initial-branch=main"]);
        replace_catalog(&repo, "v1");
        commit_all(&repo, "v1");
        let local = temp.path().join("local-bucket");
        fs::create_dir_all(&local).expect("local bucket directory");
        fs::write(
            local.join("bucket.json"),
            r#"{"schema_version":1,"name":"local","title":"Local fixture"}"#,
        )
        .expect("local bucket metadata");
        Self {
            _temp: temp,
            data,
            main_zip,
            repo,
            local,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(main_bucket::modn_binary())
            .env("LOCALAPPDATA", &self.data)
            .env(
                "MODN_MAIN_BUCKET_SOURCE",
                self.main_zip.to_str().expect("main ZIP path"),
            )
            .env(
                "MODN_TEST_MAIN_BUCKET_PUBLIC_KEY",
                main_bucket::test_public_key(),
            )
            .env_remove("MODN_GAME_MODULES_DIR")
            .args(args)
            .output()
            .expect("run fixture command")
    }

    fn success(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("fixture command UTF-8 output")
    }

    fn sync(&self) {
        let repo = self.repo.to_str().expect("repository path");
        self.success(&[
            "bucket",
            "sync",
            "community",
            "--source",
            repo,
            "--revision",
            "HEAD",
        ]);
    }

    /// Install main and register one local bucket and one signed Git catalog.
    fn ready(&self) {
        self.success(&["update"]);
        self.success(&[
            "bucket",
            "add",
            "local",
            self.local.to_str().expect("local bucket path"),
        ]);
        self.sync();
    }

    fn advance(&self, catalog: &str) -> String {
        replace_catalog(&self.repo, catalog);
        commit_all(&self.repo, catalog)
    }
}

fn json_of(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("single JSON value")
}

fn bucket<'a>(report: &'a Value, name: &str) -> &'a Value {
    report["buckets"]
        .as_array()
        .expect("bucket array")
        .iter()
        .find(|entry| entry["name"] == name)
        .unwrap_or_else(|| panic!("bucket {name} in {report}"))
}

#[test]
fn update_without_a_target_refreshes_main_and_git_and_skips_local() {
    let fixture = Fixture::new();
    fixture.ready();
    let commit = fixture.advance("v2");

    let report = json_of(&fixture.run(&["--json", "update"]));
    assert_eq!(report["failed"], 0);
    assert_eq!(report["buckets"].as_array().expect("bucket array").len(), 3);
    let main = bucket(&report, "main");
    assert_eq!(main["kind"], "official");
    assert_eq!(main["changed"], serde_json::json!(false));
    assert!(main["error"].is_null(), "{main}");
    let community = bucket(&report, "community");
    assert_eq!(community["kind"], "git");
    assert_eq!(community["changed"], serde_json::json!(true));
    assert_eq!(community["commit"], commit);
    assert_eq!(community["signer_key_id"], SIGNED_KEY);
    let local = bucket(&report, "local");
    assert_eq!(local["kind"], "local");
    assert_eq!(local["skipped"], LOCAL_SKIP);

    let output = fixture.run(&["update"]);
    let stdout = String::from_utf8(output.stdout).expect("fixture UTF-8 output");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "{stdout}");
    assert!(lines[0].starts_with("main (official):"), "{stdout}");
    assert!(
        lines[1].starts_with("community (git): changed: false"),
        "{stdout}"
    );
    assert!(
        lines[2].starts_with(&format!("local (local): skipped; {LOCAL_SKIP}")),
        "{stdout}"
    );
}

#[test]
fn a_failing_catalog_is_reported_while_the_other_buckets_refresh() {
    let fixture = Fixture::new();
    fixture.ready();
    fixture.advance("v2-rekeyed");

    let output = fixture.run(&["update"]);
    assert!(!output.status.success(), "a rekeyed catalog must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("1 of 3 catalog refreshes failed"),
        "{stderr}"
    );
    let stdout = String::from_utf8(output.stdout).expect("fixture UTF-8 output");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "{stdout}");
    assert!(
        lines[0].starts_with("main (official): changed:"),
        "{stdout}"
    );
    assert!(lines[1].starts_with("community (git): error:"), "{stdout}");
    assert!(lines[1].contains("Signer key changed"), "{stdout}");
    assert!(lines[2].contains("skipped"), "{stdout}");

    let listed = fixture.success(&["--json", "bucket", "list"]);
    let listed: Value = serde_json::from_str(&listed).expect("bucket list JSON");
    let entry = listed
        .as_array()
        .expect("bucket list array")
        .iter()
        .find(|entry| entry["name"] == "community")
        .expect("registered Git bucket");
    assert_eq!(entry["signer_key_id"], SIGNED_KEY);
}

#[test]
fn target_only_options_are_refused_without_a_target() {
    let temp = TempDir::new().expect("test fixture invariant");
    let data = temp.path().join("data");
    fs::create_dir_all(&data).expect("data directory");
    let run = |args: &[&str]| {
        Command::new(main_bucket::modn_binary())
            .env("LOCALAPPDATA", &data)
            .env_remove("MODN_GAME_MODULES_DIR")
            .args(args)
            .output()
            .expect("run fixture command")
    };
    for (args, expected) in [
        (
            &["update", "--version", "1.0"][..],
            "`--version` needs an update target",
        ),
        (
            &["update", "--file", "example.zip"][..],
            "`--file` needs an update target",
        ),
        (
            &["update", "--path", "game"][..],
            "`--path` needs an update target",
        ),
        (
            &["--game", "test-game", "update"][..],
            "`--game` needs an update target",
        ),
        (
            &["update", "--nxm", "nxm://example"][..],
            "`--nxm` needs an update target",
        ),
        (
            &["update", "--dry-run"][..],
            "`--dry-run` needs an update target",
        ),
        (
            &["update", "--no-cache"][..],
            "`--no-cache` needs an update target",
        ),
        (
            &["update", "--dry-run", "--download"][..],
            "`--dry-run` needs an update target",
        ),
        (
            &["update", "--allow-executable", "bin/tool.exe"][..],
            "`--allow-executable` needs an update target",
        ),
        (
            &["update", "--allow-native-injector"][..],
            "`--allow-native-injector` needs an update target",
        ),
    ] {
        let output = run(args);
        assert!(!output.status.success(), "{args:?} must be refused");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{args:?}: {stderr}");
    }
}
