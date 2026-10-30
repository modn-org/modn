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

#[test]
fn exact_framework_budget_does_not_report_truncation() {
    let temp = tempdir().expect("fixture");
    let game = temp.path().join("game");
    let state = temp.path().join("state");
    let marker = game.join("bin/x64/Cyberpunk2077.exe");
    fs::create_dir_all(marker.parent().expect("marker directory")).expect("game");
    fs::write(marker, b"marker").expect("game marker");
    let plugin = game.join("red4ext/plugins/Complete");
    fs::create_dir_all(&plugin).expect("plugin directory");
    for index in 0..255 {
        fs::write(plugin.join(format!("asset-{index:03}.json")), b"{}").expect("plugin asset");
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
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let report: Value = serde_json::from_slice(&result.stdout).expect("JSON");
        if command == "scan" {
            assert_eq!(
                report["incomplete"], false,
                "complete scan was marked incomplete"
            );
            assert_eq!(report["mods"].as_array().expect("mods").len(), 255);
        } else {
            assert!(!report["diagnostics"]
                .as_array()
                .expect("diagnostics")
                .iter()
                .any(|item| item["code"] == "SCAN_INCOMPLETE"));
        }
    }
}

#[test]
fn dense_redscript_inventory_returns_incomplete_instead_of_vm_timeout() {
    let temp = tempdir().expect("fixture");
    let game = temp.path().join("game");
    let state = temp.path().join("state");
    let marker = game.join("bin/x64/Cyberpunk2077.exe");
    fs::create_dir_all(marker.parent().expect("marker folder")).expect("game");
    fs::write(marker, b"game").expect("marker");
    let scripts = game.join("r6/scripts/Heavy");
    fs::create_dir_all(&scripts).expect("scripts");
    for index in 0..1500 {
        fs::write(
            scripts.join(format!("script-{index:04}.reds")),
            b"public class Fixture {}",
        )
        .expect("script");
    }
    let response = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args(["--game", "cyberpunk-2077", "--json", "--state-dir"])
        .arg(&state)
        .args(["scan", "--path"])
        .arg(&game)
        .output()
        .expect("real scan");
    assert!(
        response.status.success(),
        "{}",
        String::from_utf8_lossy(&response.stderr)
    );
    let scan: Value = serde_json::from_slice(&response.stdout).expect("JSON");
    assert_eq!(scan["incomplete"], true);
    assert!(!scan["mods"].as_array().expect("mods").is_empty());
}

#[test]
fn exact_redscript_budget_is_complete_when_no_other_framework_content_exists() {
    let temp = tempdir().expect("fixture");
    let game = temp.path().join("game");
    let state = temp.path().join("state");
    let marker = game.join("bin/x64/Cyberpunk2077.exe");
    fs::create_dir_all(marker.parent().expect("game marker parent")).expect("game");
    fs::write(marker, b"marker").expect("marker");
    let scripts = game.join("r6/scripts/Exact");
    fs::create_dir_all(&scripts).expect("scripts");
    for index in 0..255 {
        fs::write(
            scripts.join(format!("script-{index:03}.reds")),
            b"class Fixture {}",
        )
        .expect("script");
    }
    let response = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args(["--game", "cyberpunk-2077", "--json", "--state-dir"])
        .arg(&state)
        .args(["scan", "--path"])
        .arg(&game)
        .output()
        .expect("scan");
    assert!(
        response.status.success(),
        "{}",
        String::from_utf8_lossy(&response.stderr)
    );
    let scan: Value = serde_json::from_slice(&response.stdout).expect("JSON");
    assert_eq!(scan["incomplete"], false, "no files were truncated");
    assert_eq!(scan["mods"].as_array().expect("mods").len(), 255);
}

#[test]
fn exhausted_budget_checks_all_queued_directories_before_claiming_complete() {
    let temp = tempdir().expect("fixture");
    let game = temp.path().join("game");
    let state = temp.path().join("state");
    let marker = game.join("bin/x64/Cyberpunk2077.exe");
    fs::create_dir_all(marker.parent().expect("marker parent")).expect("game");
    fs::write(marker, b"marker").expect("game marker");
    let scripts = game.join("r6/scripts");
    fs::create_dir_all(scripts.join("A")).expect("nested script directory");
    fs::create_dir_all(scripts.join("Z")).expect("empty queued directory");
    for index in 0..254 {
        fs::write(
            scripts.join(format!("script-{index:03}.reds")),
            b"class Fixture {}",
        )
        .expect("script");
    }
    fs::write(scripts.join("A/hidden.reds"), b"class Hidden {}").expect("queued script");
    for command in ["scan", "doctor"] {
        let response = Command::new(env!("CARGO_BIN_EXE_modn"))
            .args(["--game", "cyberpunk-2077", "--json", "--state-dir"])
            .arg(&state)
            .arg(command)
            .arg("--path")
            .arg(&game)
            .output()
            .expect("real CLI");
        assert!(
            response.status.success(),
            "{}",
            String::from_utf8_lossy(&response.stderr)
        );
        let report: Value = serde_json::from_slice(&response.stdout).expect("JSON");
        if command == "scan" {
            assert_eq!(
                report["incomplete"], true,
                "queued nonempty directory was omitted"
            );
            assert!(!report["mods"]
                .as_array()
                .expect("mods")
                .iter()
                .any(|m| m["relative_path"] == "r6/scripts/A/hidden.reds"));
        } else {
            assert!(report["diagnostics"]
                .as_array()
                .expect("diagnostics")
                .iter()
                .any(|d| d["code"] == "SCAN_INCOMPLETE"));
        }
    }
}

#[cfg(any(unix, windows))]
fn link_file(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, link)
    }
}

#[cfg(any(unix, windows))]
#[test]
fn generated_archivexl_log_link_does_not_hide_inventory_but_blocks_adoption() {
    let temp = tempdir().expect("fixture");
    let game = temp.path().join("game");
    let state = temp.path().join("state");
    let marker = game.join("bin/x64/Cyberpunk2077.exe");
    fs::create_dir_all(marker.parent().expect("game marker parent")).expect("game");
    fs::write(marker, b"game").expect("game marker");
    let plugin = game.join("red4ext/plugins/ArchiveXL");
    fs::create_dir_all(&plugin).expect("plugin folder");
    fs::write(plugin.join("ArchiveXL.dll"), b"plugin").expect("plugin");
    let log_target = temp.path().join("plugin.log");
    fs::write(&log_target, b"generated log").expect("log target");
    if let Err(error) = link_file(&log_target, &plugin.join("ArchiveXL.log")) {
        if cfg!(windows) && error.kind() == std::io::ErrorKind::PermissionDenied {
            eprintln!("Windows test lacks symlink permission: {error}");
            return;
        }
        panic!("symlink fixture: {error}");
    }

    let run = |action: &str| {
        Command::new(env!("CARGO_BIN_EXE_modn"))
            .args(["--game", "cyberpunk-2077", "--json", "--state-dir"])
            .arg(&state)
            .arg(action)
            .arg("--path")
            .arg(&game)
            .output()
            .expect("real CLI")
    };
    let scan = run("scan");
    assert!(
        scan.status.success(),
        "{}",
        String::from_utf8_lossy(&scan.stderr)
    );
    let result: Value = serde_json::from_slice(&scan.stdout).expect("scan JSON");
    assert_eq!(
        result["incomplete"], false,
        "generated log is not missing mod content"
    );
    assert!(result["scan_issues"]
        .as_array()
        .expect("issues")
        .iter()
        .any(|item| item == "SCAN_IGNORED_GENERATED_LOG"));
    assert!(result["mods"]
        .as_array()
        .expect("mods")
        .iter()
        .any(|item| item["relative_path"] == "red4ext/plugins/ArchiveXL/ArchiveXL.dll"));
    assert!(!result["mods"]
        .as_array()
        .expect("mods")
        .iter()
        .any(|item| item["relative_path"] == "red4ext/plugins/ArchiveXL/ArchiveXL.log"));

    let doctor = run("doctor");
    assert!(
        doctor.status.success(),
        "{}",
        String::from_utf8_lossy(&doctor.stderr)
    );
    let result: Value = serde_json::from_slice(&doctor.stdout).expect("doctor JSON");
    assert!(result["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .any(|item| item["code"] == "SCAN_IGNORED_GENERATED_LOG"));
    assert!(!result["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .any(|item| item["code"] == "SCAN_INCOMPLETE"));

    let adoption = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args(["--game", "cyberpunk-2077", "--json", "--state-dir"])
        .arg(&state)
        .args(["adopt", "--dry-run", "--path"])
        .arg(&game)
        .output()
        .expect("adopt preview");
    assert!(!adoption.status.success(), "unsafe log must block adoption");
    assert!(String::from_utf8_lossy(&adoption.stderr).contains("adoption refused"));
    assert!(plugin.join("ArchiveXL.dll").is_file());
    assert!(fs::symlink_metadata(plugin.join("ArchiveXL.log"))
        .expect("log metadata")
        .file_type()
        .is_symlink());
    assert!(
        state.join("state.sqlite").is_file(),
        "scan may initialize isolated manager state"
    );
}

#[cfg(any(unix, windows))]
#[test]
fn unknown_plugin_link_reports_unsafe_entry_not_limit() {
    let temp = tempdir().expect("fixture");
    let game = temp.path().join("game");
    let marker = game.join("bin/x64/Cyberpunk2077.exe");
    fs::create_dir_all(marker.parent().expect("game marker parent")).expect("game");
    fs::write(marker, b"game").expect("game marker");
    let plugin = game.join("red4ext/plugins/Unknown");
    fs::create_dir_all(&plugin).expect("plugin folder");
    let outside = temp.path().join("unknown.asset");
    fs::write(&outside, b"untrusted").expect("target");
    if let Err(error) = link_file(&outside, &plugin.join("unknown.asset")) {
        if cfg!(windows) && error.kind() == std::io::ErrorKind::PermissionDenied {
            eprintln!("Windows test lacks symlink permission: {error}");
            return;
        }
        panic!("symlink fixture: {error}");
    }

    let scan = Command::new(env!("CARGO_BIN_EXE_modn"))
        .args(["--game", "cyberpunk-2077", "--json", "--state-dir"])
        .arg(temp.path().join("state"))
        .args(["scan", "--path"])
        .arg(&game)
        .output()
        .expect("scan");
    assert!(
        scan.status.success(),
        "{}",
        String::from_utf8_lossy(&scan.stderr)
    );
    let result: Value = serde_json::from_slice(&scan.stdout).expect("scan JSON");
    assert_eq!(result["incomplete"], true);
    assert!(result["scan_issues"]
        .as_array()
        .expect("issues")
        .iter()
        .any(|item| item == "SCAN_UNSAFE_ENTRY"));
    assert!(!result["scan_issues"]
        .as_array()
        .expect("issues")
        .iter()
        .any(|item| item == "SCAN_INCOMPLETE"));
}
