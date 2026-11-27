//! CLI tests for `modn bucket known` and a known-bucket sync.

#[path = "support/catalog_authoring.rs"]
mod catalog_authoring;
#[path = "support/main_bucket.rs"]
mod main_bucket;

use catalog_authoring::{catalog_fixture, commit_all, copy_tree, init_repo, path_text, snapshot};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

const SIGNED_KEY: &str = "2A7E5A370638B878";
const OTHER_KEY: &str = "0E250AD52A3BA2AA";
const KNOWN_URL: &str = "https://example.invalid/community.git";

struct Fixture {
    temp: TempDir,
    data: PathBuf,
    state: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().expect("test fixture invariant");
        let data = temp.path().join("data");
        fs::create_dir_all(&data).expect("data directory");
        main_bucket::seed(&data);
        let state = temp.path().join("state");
        main_bucket::seed_state(&state);
        Self { temp, data, state }
    }

    fn write_known(&self, entries: &str) {
        fs::write(
            self.state.join("buckets/main/known-buckets.json"),
            entries,
        )
        .expect("known bucket list");
    }

    fn run(&self, variables: &[(&str, &str)], args: &[&str]) -> Output {
        run_in(&self.data, &self.state, variables, args)
    }

    fn run_json(&self, variables: &[(&str, &str)], args: &[&str]) -> Output {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        self.run(variables, &all)
    }

    fn registration(&self, name: &str) -> PathBuf {
        self.state
            .join("buckets/registrations")
            .join(format!("{name}.json"))
    }
}

fn run_in(data: &Path, state: &Path, variables: &[(&str, &str)], args: &[&str]) -> Output {
    let mut command = Command::new(main_bucket::modn_binary());
    command
        .env("LOCALAPPDATA", data)
        .env("APPDATA", data)
        .env("MODN_APPDATA", data)
        .env_remove("MODN_GAME_MODULES_DIR")
        .arg("--state-dir")
        .arg(state)
        .args(args);
    for (name, value) in variables {
        command.env(name, value);
    }
    command.output().expect("run modn")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout)
        .replace("\r\n", "\n")
        .trim_end()
        .to_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn signed_repo(root: &Path) -> PathBuf {
    let repo = init_repo(root);
    copy_tree(&catalog_fixture("v1"), &repo);
    commit_all(&repo, "v1");
    repo
}

fn entry(name: &str, kind: &str, revision: Option<&str>, fingerprint: &str) -> String {
    let suffix = if kind == "zip" { "zip" } else { "git" };
    let mut fields = format!(
        r#""kind":"{kind}","source":"https://example.invalid/{name}.{suffix}","fingerprint":"{fingerprint}","description":"{name} bucket.""#
    );
    if let Some(revision) = revision {
        fields.push_str(&format!(r#","revision":"{revision}""#));
    }
    format!(r#""{name}":{{{fields}}}"#)
}

fn known_list(entries: &[String]) -> String {
    format!(
        r#"{{"schema_version":1,"buckets":{{{}}}}}"#,
        entries.join(",")
    )
}

fn redirect<'a>(repo: &'a Path) -> Vec<(&'static str, &'a str)> {
    vec![
        ("MODN_TEST_KNOWN_BUCKET_SOURCE", KNOWN_URL),
        ("MODN_TEST_KNOWN_BUCKET_TARGET", path_text(repo)),
    ]
}

#[test]
fn known_lists_entries_in_name_order_with_descriptions() {
    let fixture = Fixture::new();
    fixture.write_known(&known_list(&[
        entry("zeta", "zip", None, OTHER_KEY),
        entry("alpha", "git", Some("main"), SIGNED_KEY),
    ]));

    let output = fixture.run(&[], &["bucket", "known"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        concat!(
            "alpha  git  2A7E5A370638B878  https://example.invalid/alpha.git\n",
            "  alpha bucket.\n",
            "zeta  zip  0E250AD52A3BA2AA  https://example.invalid/zeta.zip\n",
            "  zeta bucket.\n",
            "Sync one with: modn bucket sync <name>",
        )
    );

    let json = fixture.run_json(&[], &["bucket", "known"]);
    assert!(json.status.success(), "{}", stderr(&json));
    let value: Value = serde_json::from_slice(&json.stdout).expect("known JSON");
    let object = value.as_object().expect("known object");
    assert_eq!(object.len(), 1);
    let buckets = value["buckets"].as_array().expect("bucket array");
    let names: Vec<&str> = buckets
        .iter()
        .map(|bucket| bucket["name"].as_str().expect("bucket name"))
        .collect();
    assert_eq!(names, ["alpha", "zeta"]);
    assert_eq!(buckets[0]["kind"], "git");
    assert_eq!(buckets[0]["source"], "https://example.invalid/alpha.git");
    assert_eq!(buckets[0]["revision"], "main");
    assert_eq!(buckets[0]["description"], "alpha bucket.");
    assert_eq!(buckets[0]["fingerprint"], SIGNED_KEY);
    assert!(buckets[1]["revision"].is_null());
}

#[test]
fn known_reports_an_empty_or_missing_list() {
    let fixture = Fixture::new();

    let empty = fixture.run(&[], &["bucket", "known"]);
    assert!(empty.status.success(), "{}", stderr(&empty));
    assert_eq!(stdout(&empty), "main lists no known buckets");

    let empty_json = fixture.run_json(&[], &["bucket", "known"]);
    assert!(empty_json.status.success(), "{}", stderr(&empty_json));
    let value: Value = serde_json::from_slice(&empty_json.stdout).expect("known JSON");
    assert_eq!(value, serde_json::json!({"buckets": []}));

    let missing_state = fixture.temp.path().join("missing-state");
    let missing = run_in(&fixture.data, &missing_state, &[], &["bucket", "known"]);
    assert!(missing.status.success(), "{}", stderr(&missing));
    assert!(stdout(&missing).contains("main lists no known buckets"));
    assert!(stdout(&missing).contains("modn bucket update"));
}

#[test]
fn known_sync_registers_the_catalog_without_module_trust() {
    let fixture = Fixture::new();
    let repo = signed_repo(fixture.temp.path());
    fixture.write_known(&known_list(&[entry(
        "community",
        "git",
        Some("main"),
        SIGNED_KEY,
    )]));

    let output = fixture.run(&redirect(&repo), &["bucket", "sync", "community"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("Synced bucket community at commit"), "{text}");
    assert!(text.contains("module trust was not granted"), "{text}");
    assert!(
        text.contains("modn bucket trust-modules community"),
        "{text}"
    );

    let registration: Value = serde_json::from_slice(
        &fs::read(fixture.registration("community")).expect("registration"),
    )
    .expect("registration JSON");
    assert_eq!(registration["git"]["source"]["remote"], path_text(&repo));
    assert_eq!(registration["git"]["signer"]["key_id"], SIGNED_KEY);
    assert!(
        registration.get("module_trust").is_none(),
        "module trust was recorded"
    );

    let listed = fixture.run_json(&[], &["bucket", "list"]);
    assert!(listed.status.success(), "{}", stderr(&listed));
    let entries: Value = serde_json::from_slice(&listed.stdout).expect("list JSON");
    let community = entries
        .as_array()
        .expect("list array")
        .iter()
        .find(|bucket| bucket["name"] == "community")
        .expect("registered known bucket");
    assert_eq!(community["source_kind"], "git");
    assert_eq!(community["module_trusted"], false);
}

#[test]
fn known_sync_refuses_a_fingerprint_mismatch_without_writing_state() {
    let fixture = Fixture::new();
    let repo = signed_repo(fixture.temp.path());
    fixture.write_known(&known_list(&[entry(
        "community",
        "git",
        Some("main"),
        "0000000000000000",
    )]));
    let warm = fixture.run(
        &[],
        &[
            "bucket",
            "sync",
            "community",
            "--source",
            path_text(&fixture.temp.path().join("missing-repo")),
            "--revision",
            "main",
        ],
    );
    assert!(!warm.status.success());
    assert!(!fixture.registration("community").exists());
    let before = snapshot(&fixture.state);

    let refused = fixture.run_json(&redirect(&repo), &["bucket", "sync", "community"]);
    assert!(!refused.status.success());
    let error: Value = serde_json::from_slice(&refused.stderr).expect("error JSON");
    assert_eq!(error["error"], "BUCKET_FINGERPRINT_MISMATCH");
    let message = error["message"].as_str().expect("error message");
    assert!(message.contains("0000000000000000"), "{message}");
    assert!(message.contains(SIGNED_KEY), "{message}");
    assert!(message.contains("nothing was written"), "{message}");

    assert_eq!(snapshot(&fixture.state), before);
    assert!(!fixture.registration("community").exists());
}

#[test]
fn unknown_or_unregistered_sync_names_the_known_command() {
    let fixture = Fixture::new();
    fixture.write_known(&known_list(&[entry(
        "community",
        "git",
        Some("main"),
        SIGNED_KEY,
    )]));

    let refused = fixture.run_json(&[], &["bucket", "sync", "ghost"]);
    assert!(!refused.status.success());
    let error: Value = serde_json::from_slice(&refused.stderr).expect("error JSON");
    assert_eq!(error["error"], "BUCKET_FAILED");
    assert!(
        error["message"]
            .as_str()
            .expect("error message")
            .contains("modn bucket known lists the known buckets")
    );
}

#[test]
fn revision_without_source_on_a_known_name_is_a_usage_error() {
    let fixture = Fixture::new();
    fixture.write_known(&known_list(&[entry(
        "community",
        "git",
        Some("main"),
        SIGNED_KEY,
    )]));

    let refused = fixture.run_json(&[], &["bucket", "sync", "community", "--revision", "main"]);
    assert!(!refused.status.success());
    let error: Value = serde_json::from_slice(&refused.stderr).expect("error JSON");
    assert!(
        error["message"]
            .as_str()
            .expect("error message")
            .contains("`--revision` needs `--source`")
    );
    assert!(!fixture.registration("community").exists());
}
