use super::support::{
    assert_bytes_have_no_secrets, expires, free_responses, nxm_link, scan_state_tree, with_api_key,
    Fixture, CDN_URL, FILE_ID,
};
use crate::download::nexus_download;
use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn free_path_keeps_secrets_out_of_output_and_state() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path().join("manager");
    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .arg("--exact")
        .arg("nexus_fixture_tests::secrecy::nexus_fixture_child")
        .arg("--ignored")
        .arg("--nocapture")
        .env("MODN_NEXUS_FIXTURE_ROOT", &root)
        .output()
        .expect("run fixture child");
    assert!(
        output.status.success(),
        "child failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut text = output.stdout;
    text.extend_from_slice(&output.stderr);
    assert_bytes_have_no_secrets(&text, Path::new("<child output>"));
    let output_text = String::from_utf8_lossy(&text);
    assert!(
        output_text.contains("Nexus Mods: cyberpunk2077 mod 6945 file 123456 (NexusMod.zip)"),
        "{output_text}"
    );
    let cache = root.join("cache");
    assert!(
        fs::read_dir(&cache)
            .expect("cache directory")
            .filter_map(std::result::Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().contains('#')),
        "the free-path install wrote a cache entry"
    );
    assert!(root.join("state.sqlite").is_file());
    scan_state_tree(&root);
}

#[test]
#[ignore = "spawned by free_path_keeps_secrets_out_of_output_and_state"]
fn nexus_fixture_child() {
    let Some(root) = std::env::var_os("MODN_NEXUS_FIXTURE_ROOT").map(PathBuf::from) else {
        return;
    };
    let fixture = Fixture::at(root, b"v1");
    let expires = expires();
    nexus_download::set_responses(free_responses(
        FILE_ID,
        expires,
        CDN_URL,
        &fixture.archive_bytes("1"),
    ));
    with_api_key(|| {
        fixture.run(&[
            "install",
            "nexusmod",
            "--path",
            fixture.path(),
            "--yes",
            "--nxm",
            &nxm_link(FILE_ID, expires),
        ])
    })
    .expect("fixture install");
}
