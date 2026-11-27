#![allow(
    dead_code,
    reason = "shared fixture exposes helpers for different test crates"
)]
use crate::catalog_authoring::{json, path_text, prepare_catalog, run, stderr};
use crate::zip_catalog::snapshot;
use crate::zip_catalog_fixture::Fixture;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

/// Write one signed catalog and return its directory.
pub fn numbered_catalog(
    root: &Path,
    name: &str,
    keys: &Path,
    key_name: &str,
    sequence: &str,
    expires: Option<&str>,
    extra: &[(&str, &str)],
) -> PathBuf {
    let catalog = prepare_catalog(root, name);
    for (file, body) in extra {
        fs::write(catalog.join(file), body).expect("catalog metadata");
    }
    let output = sign(
        &catalog,
        &keys.join(format!("{key_name}.key")),
        Some(sequence),
        expires,
    );
    assert!(
        output.status.success(),
        "numbered sign failed: {}",
        stderr(&output)
    );
    catalog
}

/// Run `modn catalog sign` with optional sequence flags.
pub fn sign(catalog: &Path, key: &Path, sequence: Option<&str>, expires: Option<&str>) -> Output {
    let mut arguments = vec![
        "--json".to_owned(),
        "catalog".to_owned(),
        "sign".to_owned(),
        path_text(catalog).to_owned(),
        "--key".to_owned(),
        path_text(key).to_owned(),
    ];
    if let Some(sequence) = sequence {
        arguments.push("--sequence".to_owned());
        arguments.push(sequence.to_owned());
    }
    if let Some(expires) = expires {
        arguments.push("--expires".to_owned());
        arguments.push(expires.to_owned());
    }
    let borrowed: Vec<&str> = arguments.iter().map(String::as_str).collect();
    run(&borrowed)
}

/// Write a raw `sequence.json`, usually to build an expired or malformed case.
pub fn write_sequence(catalog: &Path, body: &str) {
    fs::write(catalog.join("sequence.json"), body).expect("sequence file");
}

/// Read one JSON file from the catalog, such as `rotation.json`.
pub fn catalog_json(catalog: &Path, name: &str) -> Value {
    serde_json::from_slice(&fs::read(catalog.join(name)).expect("catalog JSON file"))
        .expect("catalog JSON")
}

pub fn statement_sequence(catalog: &Path, name: &str) -> Option<u64> {
    catalog_json(catalog, name)["sequence"].as_u64()
}

pub fn error_code(output: &Output) -> String {
    let value: Value = serde_json::from_slice(&output.stderr).expect("error JSON");
    value["error"].as_str().expect("error code").to_owned()
}

/// Parse the JSON object that a successful command printed.
pub fn printed(output: &Output) -> Value {
    json(output)
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("command UTF-8 output")
}

pub fn stderr_text(output: &Output) -> String {
    stderr(output)
}

/// Bytes of the registration and the published tree, to prove a refusal changed nothing.
pub fn state_snapshot(fixture: &Fixture) -> (Vec<u8>, Vec<(String, Vec<u8>)>) {
    (
        fs::read(fixture.registration()).expect("registration bytes"),
        snapshot(&fixture.catalog()),
    )
}

/// Run one `bucket sync community` against a ZIP source.
pub fn sync(fixture: &Fixture, zip: &Path, json_output: bool) -> Output {
    if json_output {
        fixture.run(&[
            "--json",
            "bucket",
            "sync",
            "community",
            "--source",
            path_text(zip),
        ])
    } else {
        fixture.run(&["bucket", "sync", "community", "--source", path_text(zip)])
    }
}
