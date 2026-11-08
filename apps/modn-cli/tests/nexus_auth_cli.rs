#[path = "support/main_bucket.rs"]
mod main_bucket;
use std::fs;
use tempfile::tempdir;

const KEY: &str = "NEXUS_CLI_SECRET_4321";

#[test]
fn status_reports_the_environment_key_without_printing_it() {
    let output = main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .env("MODN_NEXUS_API_KEY", KEY)
        .args(["auth", "nexus", "status"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim(), "Nexus API key: set (MODN_NEXUS_API_KEY)");
    assert!(!stdout.contains(KEY));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(KEY));
}

#[test]
fn install_and_update_refuse_a_bad_nxm_link_before_reading_state() {
    let temp = tempdir().unwrap();
    let state = temp.path().join("state");
    let state_str = state.to_str().unwrap();
    let link = format!("nxm://cyberpunk2077/mods/107/files/123?key={KEY}&expires=1");
    for command in ["install", "update"] {
        let output = main_bucket::command(env!("CARGO_BIN_EXE_modn"))
            .args([
                "--state-dir",
                state_str,
                command,
                "cet",
                "--nxm",
                &link,
                "--yes",
            ])
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "{command} accepted an expired link"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("expired"), "{stderr}");
        assert!(!stderr.contains(KEY), "{stderr}");
        assert!(!stderr.contains("expires=1"), "{stderr}");
        assert!(!stderr.contains("nxm://"), "{stderr}");
        assert!(!state.join("state.sqlite").exists(), "{command} read state");
    }
}

#[test]
fn install_refuses_a_link_with_an_unknown_parameter() {
    let temp = tempdir().unwrap();
    let state = temp.path().join("state");
    let link =
        format!("nxm://cyberpunk2077/mods/107/files/123?key={KEY}&expires=9999999999&extra=1");
    let output = main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "install",
            "cet",
            "--nxm",
            &link,
            "--yes",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown query parameter"), "{stderr}");
    assert!(!stderr.contains(KEY), "{stderr}");
    assert!(!stderr.contains("9999999999"), "{stderr}");
    assert!(fs::metadata(state.join("state.sqlite")).is_err());
}
