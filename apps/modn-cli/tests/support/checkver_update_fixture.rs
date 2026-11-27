#![allow(
    dead_code,
    reason = "shared fixture exposes helpers for different test crates"
)]

use crate::checkver_fixture::{github_route, stderr, Fixture, MockServer};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub const REMINDER: &str = "Run 'modn catalog sign' and 'modn catalog verify' before you publish; checkver never changes catalog.sig.";

pub fn zeros() -> String {
    "0".repeat(64)
}

pub fn manifest_text(
    id: &str,
    version: &str,
    tag: &str,
    asset: &str,
    blake3: &str,
    upstream_sha256: Option<&str>,
) -> String {
    let upstream = upstream_sha256
        .map(|sha| format!("  \"upstream_sha256\": \"{sha}\",\n"))
        .unwrap_or_default();
    format!(
        "{{\n  \"name\": \"Fixture {id}\",\n  \"schema_version\": 2,\n  \"id\": \"{id}\",\n  \"game\": \"test-game\",\n  \"version\": \"{version}\",\n  \"archive_format\": \"zip\",\n  \"blake3\": \"{blake3}\",\n{upstream}  \"sources\": [\n    {{\n      \"kind\": \"github-release\",\n      \"repository\": \"owner/{id}\",\n      \"tag\": \"{tag}\",\n      \"asset\": \"{asset}\"\n    }}\n  ],\n  \"dependencies\": [],\n  \"checkver\": {{\n    \"kind\": \"github-release\",\n    \"repository\": \"owner/{id}\",\n    \"asset_pattern\": \"*\"\n  }}\n}}\n"
    )
}

pub fn add_release(
    routes: &mut BTreeMap<String, (u16, Vec<u8>)>,
    id: &str,
    tag: &str,
    asset: &str,
    bytes: Option<&[u8]>,
) {
    let (path, route) = github_route(
        &format!("owner/{id}"),
        json!([{"tag_name": tag, "assets": [{"name": asset}]}]),
    );
    routes.insert(path, route);
    if let Some(bytes) = bytes {
        routes.insert(
            format!("/owner/{id}/releases/download/{tag}/{asset}"),
            (200, bytes.to_vec()),
        );
    }
}

pub fn server_env(server: &MockServer) -> Vec<(&'static str, &str)> {
    vec![
        ("MODN_TEST_CHECKVER_GITHUB_API", server.address.as_str()),
        ("MODN_TEST_CHECKVER_GITHUB_HOST", server.address.as_str()),
    ]
}

pub fn directory_entries(path: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(path)
        .expect("directory")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

pub fn sign_and_verify(fixture: &Fixture) {
    let keys = fixture.temp.path().join("keys");
    let keygen = fixture.run_plain(
        &[],
        &[
            "catalog",
            "keygen",
            "--out",
            keys.to_str().expect("keys path"),
        ],
    );
    assert!(keygen.status.success(), "{}", stderr(&keygen));
    let key = keys.join("catalog.key");
    let public = keys.join("catalog.pub");
    let sign = fixture.run(
        &[],
        &[
            "catalog",
            "sign",
            "--key",
            key.to_str().expect("secret key path"),
        ],
    );
    assert!(sign.status.success(), "{}", stderr(&sign));
    let verify = fixture.run(
        &[],
        &[
            "catalog",
            "verify",
            "--key",
            public.to_str().expect("public key path"),
        ],
    );
    assert!(verify.status.success(), "{}", stderr(&verify));
}
