#[path = "support/main_bucket.rs"]
mod main_bucket;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn cli(root: &Path, input: &Path, json: bool) -> Output {
    let mut command = Command::new(main_bucket::modn_binary());
    command
        .env("LOCALAPPDATA", root.join("data"))
        .env("MODN_GAME_MODULES_DIR", root.join("missing-modules"))
        .args([
            "--state-dir",
            root.join("state").to_str().expect("UTF-8 fixture path"),
        ])
        .args(["--game", "missing-game", "--module-src", "missing-source"])
        .args(["asset", "inspect"])
        .arg(input);
    if json {
        command.arg("--json");
    }
    command.output().expect("run asset CLI")
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let mut result = BTreeMap::new();
    for entry in fs::read_dir(root).expect("read fixture tree") {
        let path = entry.expect("fixture entry").path();
        if path.is_dir() {
            result.insert(path.clone(), None);
            result.extend(snapshot(&path));
        } else {
            result.insert(
                path.clone(),
                Some(fs::read(path).expect("read fixture file")),
            );
        }
    }
    result
}

fn dds() -> Vec<u8> {
    let mut bytes = vec![0; 128];
    bytes[..4].copy_from_slice(b"DDS ");
    for (offset, value) in [(4, 124_u32), (12, 32), (16, 64), (76, 32), (80, 4)] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[84..88].copy_from_slice(b"DXT1");
    bytes
}

fn redengine() -> Vec<u8> {
    let mut bytes = vec![0; 200];
    bytes[..4].copy_from_slice(b"RDAR");
    bytes[4..8].copy_from_slice(&12_u32.to_le_bytes());
    bytes[8..16].copy_from_slice(&172_u64.to_le_bytes());
    bytes[16..20].copy_from_slice(&28_u32.to_le_bytes());
    bytes[32..40].copy_from_slice(&200_u64.to_le_bytes());
    bytes[172..176].copy_from_slice(&8_u32.to_le_bytes());
    bytes[176..180].copy_from_slice(&20_u32.to_le_bytes());
    bytes
}

#[test]
fn assets_report_typed_metadata_without_game_or_state_access() {
    let fixture = tempfile::tempdir().unwrap();
    for (name, bytes, format, human) in [
        ("texture.bin", dds(), "dds", "64 x 32"),
        ("archive.bin", redengine(), "redengine", "version 12"),
    ] {
        let input = fixture.path().join(name);
        fs::write(&input, &bytes).unwrap();
        for existing_state in [false, true] {
            if existing_state {
                fs::create_dir_all(fixture.path().join("state")).unwrap();
                fs::write(
                    fixture.path().join("state/state.sqlite"),
                    b"invalid database",
                )
                .unwrap();
                fs::create_dir_all(fixture.path().join("data")).unwrap();
                fs::write(fixture.path().join("data/keep"), b"manual").unwrap();
            }
            let before = snapshot(fixture.path());
            for json in [true, false] {
                let output = cli(fixture.path(), &input, json);
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                if json {
                    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
                    assert_eq!(report["format"], format);
                    assert_eq!(report["bytes"], bytes.len());
                    if format == "dds" {
                        assert_eq!(report["metadata"]["width"], 64);
                        assert_eq!(report["metadata"]["height"], 32);
                        assert_eq!(report["metadata"]["pixel_format"]["compression"], "bc1");
                    } else {
                        assert_eq!(report["metadata"]["version"], 12);
                        assert_eq!(report["metadata"]["file_count"], 0);
                        assert_eq!(report["metadata"]["compression"], "unknown");
                    }
                } else {
                    let text = String::from_utf8(output.stdout).unwrap();
                    assert!(text.contains(human), "{text}");
                    assert!(text.contains("structural metadata only"), "{text}");
                    assert!(text.contains(&format!("{} bytes", bytes.len())), "{text}");
                }
                assert_eq!(snapshot(fixture.path()), before);
            }
        }
    }
}

#[test]
fn malformed_and_unsupported_assets_fail_without_writes() {
    let fixture = tempfile::tempdir().unwrap();
    let mut invalid_dds = dds();
    invalid_dds[4..8].copy_from_slice(&0_u32.to_le_bytes());
    let mut unsupported = redengine();
    unsupported[4..8].copy_from_slice(&11_u32.to_le_bytes());
    let mut invalid_index = redengine();
    invalid_index[16..20].copy_from_slice(&29_u32.to_le_bytes());
    for (bytes, reason) in [
        (b"DDS ".to_vec(), "truncated"),
        (b"unknown".to_vec(), "unsupported asset signature"),
        (invalid_dds, "invalid DDS"),
        (unsupported, "unsupported REDengine archive version 11"),
        (invalid_index, "outside the file"),
    ] {
        let input = fixture.path().join("asset");
        fs::write(&input, bytes).unwrap();
        let before = snapshot(fixture.path());
        for json in [true, false] {
            let output = cli(fixture.path(), &input, json);
            assert!(!output.status.success());
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(error.contains(reason), "{error}");
            assert_eq!(snapshot(fixture.path()), before);
        }
    }
    for input in [fixture.path().to_path_buf(), fixture.path().join("missing")] {
        let before = snapshot(fixture.path());
        assert!(!cli(fixture.path(), &input, true).status.success());
        assert_eq!(snapshot(fixture.path()), before);
    }
}

#[test]
fn help_describes_asset_inspection() {
    let fixture = tempfile::tempdir().unwrap();
    let output = Command::new(main_bucket::modn_binary())
        .env("LOCALAPPDATA", fixture.path())
        .args(["asset", "inspect", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("DDS") && text.contains("RDAR"), "{text}");
    assert!(text.contains("--json"), "{text}");
    assert!(snapshot(fixture.path()).is_empty());
}
