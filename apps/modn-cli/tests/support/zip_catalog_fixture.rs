#![allow(
    dead_code,
    reason = "shared fixture exposes helpers for different test crates"
)]
use crate::main_bucket;
use crate::zip_catalog;
use crate::zip_catalog::{listing, pack, snapshot};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;
pub type StateSnapshot = (Vec<u8>, Vec<(String, Vec<u8>)>, Vec<String>);
pub struct Fixture {
    pub temp: TempDir,
    data: PathBuf,
    state: PathBuf,
    main_zip: PathBuf,
}
impl Fixture {
    pub fn new() -> Self {
        let temp = TempDir::new().expect("test fixture invariant");
        let data = temp.path().join("data");
        fs::create_dir_all(&data).expect("data directory");
        main_bucket::seed(&data);
        let tree = temp.path().join("main-tree");
        zip_catalog::copy_catalog(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main"),
            &tree,
        );
        let main_zip = temp.path().join("main.zip");
        pack(&tree, &main_zip);
        main_bucket::write_signature(&tree, &main_zip);
        let state = temp.path().join("state");
        Self {
            temp,
            data,
            state,
            main_zip,
        }
    }
    pub fn run(&self, args: &[&str]) -> Output {
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
            .args(["--state-dir", self.state.to_str().expect("state path")])
            .args(args)
            .output()
            .expect("run fixture command")
    }
    pub fn success(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("fixture command UTF-8 output")
    }
    pub fn sync(&self, zip: &Path) -> Output {
        self.run(&[
            "--json",
            "bucket",
            "sync",
            "community",
            "--source",
            zip.to_str().expect("ZIP path"),
        ])
    }
    pub fn registration(&self) -> PathBuf {
        self.state.join("buckets/registrations/community.json")
    }
    pub fn bucket_entries(&self) -> Vec<String> {
        listing(&self.state.join("buckets"))
    }
    pub fn catalog(&self) -> PathBuf {
        let bytes = fs::read(self.registration()).expect("registration file");
        let value: Value = serde_json::from_slice(&bytes).expect("registration JSON");
        PathBuf::from(value["path"].as_str().expect("registration path"))
    }
    pub fn state_snapshot(&self) -> StateSnapshot {
        (
            fs::read(self.registration()).expect("registration file"),
            snapshot(&self.catalog()),
            listing(&self.state.join("buckets")),
        )
    }
    pub fn assert_no_staging(&self) {
        let buckets = self.state.join("buckets");
        assert!(
            !listing(&buckets)
                .iter()
                .any(|name| name.starts_with("zip-catalog-")),
            "staging left behind: {:?}",
            listing(&buckets)
        );
    }
}
pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}
pub fn listed(fixture: &Fixture) -> Vec<Value> {
    let output = fixture.success(&["--json", "bucket", "list"]);
    serde_json::from_str(&output).expect("bucket list JSON")
}
pub fn assert_refused_without_state(fixture: &Fixture, output: &Output, expected: &str) {
    assert!(!output.status.success(), "command unexpectedly succeeded");
    assert!(
        stderr(output).contains(expected),
        "expected {expected:?} in {}",
        stderr(output)
    );
    assert!(!fixture.registration().exists(), "registration was written");
    fixture.assert_no_staging();
}
