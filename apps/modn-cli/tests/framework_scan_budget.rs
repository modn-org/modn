//! Real CLI regression for dense framework inventories.
use serde_json::Value;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn dense_cet_assets_produce_incomplete_inventory_without_vm_timeout() {
    let temp = tempdir().expect("fixture");
    let game = temp.path().join("game");
    let state = temp.path().join("state");
    let marker = game.join("bin/x64/Cyberpunk2077.exe");
    fs::create_dir_all(marker.parent().expect("marker directory")).expect("game");
    fs::write(marker, b"marker").expect("game marker");
    let assets = game.join("bin/x64/plugins/cyber_engine_tweaks/mods/Heavy/assets");
    fs::create_dir_all(&assets).expect("assets");
    for index in 0..1500 {
        fs::write(assets.join(format!("asset-{index:04}.json")), b"{}").expect("asset");
    }

    for command in ["scan", "doctor"] {
        let result = Command::new(env!("CARGO_BIN_EXE_modn"))
            .args(["--game", "cyberpunk-2077", "--json", "--state-dir"])
            .arg(&state)
            .arg(command)
            .arg("--path")
            .arg(&game)
            .output()
            .expect("real CLI");
        assert!(
            result.status.success(),
            "{command} failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let report: Value = serde_json::from_slice(&result.stdout).expect("JSON");
        if command == "scan" {
            assert_eq!(report["incomplete"], true, "must disclose truncated scan");
            assert!(!report["mods"].as_array().expect("mods").is_empty());
        } else {
            assert!(report["diagnostics"]
                .as_array()
                .expect("diagnostics")
                .iter()
                .any(|item| item["code"] == "SCAN_INCOMPLETE"));
        }
    }
}
