use mod_core::scan::{doctor, scan};
use std::fs;
use tempfile::tempdir;

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

    let report = scan(root);
    assert!(report.redmod_available);
    assert_eq!(report.mods.len(), 6);
    assert!(report.mods.iter().all(|entry| !entry.managed));
    assert!(report.mods.iter().any(|entry| entry.kind == "legacy-archive"));
    assert!(report.mods.iter().any(|entry| entry.kind == "redmod-directory"));
    assert!(report.mods.iter().any(|entry| entry.kind == "cet-mod"));
    assert!(doctor(root).diagnostics.iter().any(|d| d.code == "UNMANAGED_CONTENT"));
    for file in files {
        assert_eq!(fs::read_to_string(root.join(file)).unwrap(), "unchanged");
    }
}

#[test]
fn missing_redmod_tools_warns_without_failing_scan() {
    let temp = tempdir().unwrap();
    fs::create_dir_all(temp.path().join("mods/Test")).unwrap();
    let result = doctor(temp.path());
    assert!(result.diagnostics.iter().any(|d| d.code == "REDMOD_CONTENT_WITHOUT_TOOLS"));
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
    assert!(scan(temp.path()).mods.is_empty());
}
