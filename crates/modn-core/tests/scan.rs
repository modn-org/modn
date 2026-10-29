//! Integration checks for scan behavior.
use modn_core::scan::{doctor_for, scan_for};
use std::fs;
use tempfile::tempdir;

#[test]
fn directory_heavy_scan_preserves_every_mod() {
    let temp = tempdir().unwrap();
    let base = temp.path().join("archive/pc/mod");
    for number in 0..141 {
        let folder = base.join(format!("folder-{number:03}"));
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("example.archive"), b"fixture").unwrap();
    }

    let report = scan_for("cyberpunk-2077", temp.path());
    assert!(!report.incomplete, "scan must not silently truncate");
    assert_eq!(report.mods.len(), 141);
}

#[test]
fn nested_unreadable_depth_marks_scan_incomplete() {
    let temp = tempdir().unwrap();
    let mut nested = temp.path().join("archive/pc/mod");
    for number in 0..18 {
        nested = nested.join(format!("d{number:02}"));
    }
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("deep.archive"), b"fixture").unwrap();

    let report = scan_for("cyberpunk-2077", temp.path());
    assert!(
        report.incomplete,
        "failed nested directory read must be reported"
    );
}

#[test]
fn reports_known_layouts_without_modifying_files() {
    let temp = tempdir().unwrap();
    let root = temp.path();
    let files = [
        "archive/pc/mod/example.archive",
        "r6/scripts/nested/a.reds",
        "r6/tweaks/settings.yaml",
        "mods/MyRedmod/info.json",
        "red4ext/plugins/Codeware/entry.dll",
        "bin/x64/plugins/cyber_engine_tweaks/mods/AMM/init.lua",
    ];
    for file in files {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "unchanged").unwrap();
    }
    fs::create_dir_all(root.join("tools/redmod/bin")).unwrap();
    fs::write(root.join("tools/redmod/bin/redMod.exe"), "tool").unwrap();

    let report = scan_for("cyberpunk-2077", root);
    assert!(report.redmod_available);
    assert_eq!(report.mods.len(), 6);
    assert!(report.mods.iter().all(|entry| !entry.managed));
    assert!(report
        .mods
        .iter()
        .any(|entry| entry.kind == "legacy-archive"));
    assert!(report
        .mods
        .iter()
        .any(|entry| entry.kind == "redmod-directory"));
    assert!(report.mods.iter().any(|entry| entry.kind == "cet-mod"));
    assert!(doctor_for("cyberpunk-2077", root)
        .diagnostics
        .iter()
        .any(|d| d.code == "UNMANAGED_CONTENT"));
    for file in files {
        assert_eq!(fs::read_to_string(root.join(file)).unwrap(), "unchanged");
    }
}

#[test]
fn missing_redmod_tools_warns_without_failing_scan() {
    let temp = tempdir().unwrap();
    fs::create_dir_all(temp.path().join("mods/Test")).unwrap();
    let result = doctor_for("cyberpunk-2077", temp.path());
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.code == "REDMOD_CONTENT_WITHOUT_TOOLS"));
}

#[cfg(unix)]
#[test]
fn does_not_traverse_symlinked_mod_directories() {
    use std::os::unix::fs::symlink;
    let temp = tempdir().unwrap();
    let outside = temp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("bad.archive"), "outside").unwrap();
    fs::create_dir_all(temp.path().join("archive/pc")).unwrap();
    symlink(&outside, temp.path().join("archive/pc/mod")).unwrap();
    assert!(scan_for("cyberpunk-2077", temp.path()).mods.is_empty());
}
