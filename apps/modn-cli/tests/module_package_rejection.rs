//! Reject hostile packages before any persistent state is created.
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use zip::write::SimpleFileOptions;

const MANIFEST: &str = r#"{"package_format":1,"game_id":"test-game","version":"1.0","api_version":1,"entrypoint":"main.luau"}"#;
const SCRIPT: &str = "return {game_id='test-game',api_version=1}";

fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zip.start_file(
            *name,
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
        )
        .expect("fixture operation");
        zip.write_all(bytes).expect("fixture operation");
    }
    zip.finish().expect("fixture operation").into_inner()
}

fn reject(root: &Path, label: &str, bytes: &[u8]) {
    let path = root.join("hostile.zip");
    let state = root.join("state");
    fs::write(&path, bytes).expect("fixture operation");
    for flags in ["--dry-run", "--yes"] {
        let output = Command::new(env!("CARGO_BIN_EXE_modn"))
            .args(["--state-dir"])
            .arg(&state)
            .args(["module", "install"])
            .arg(&path)
            .arg(flags)
            .output()
            .expect("fixture operation");
        assert!(!output.status.success(), "accepted {label}");
        assert!(!state.exists(), "{label} created state");
        assert!(!output.stderr.is_empty(), "{label} lacks explicit error");
    }
}

#[test]
fn incompatible_manifest_and_malformed_source_never_create_state() {
    let t = tempfile::tempdir().expect("fixture operation");
    for (label, manifest, script) in [
        (
            "future API",
            MANIFEST.replace("\"api_version\":1", "\"api_version\":2"),
            SCRIPT.as_bytes(),
        ),
        (
            "future package",
            MANIFEST.replace("\"package_format\":1", "\"package_format\":2"),
            SCRIPT.as_bytes(),
        ),
        (
            "legacy unpackaged",
            MANIFEST.replace("\"package_format\":1,", ""),
            SCRIPT.as_bytes(),
        ),
        (
            "unsafe game",
            MANIFEST.replace("test-game", "../test-game"),
            SCRIPT.as_bytes(),
        ),
        (
            "entrypoint traversal",
            MANIFEST.replace("main.luau", "../main.luau"),
            SCRIPT.as_bytes(),
        ),
        (
            "wrong runtime identity",
            MANIFEST.into(),
            b"return {game_id='other-game',api_version=1}",
        ),
        (
            "wrong runtime API",
            MANIFEST.into(),
            b"return {game_id='test-game',api_version=2}",
        ),
        (
            "wrong runtime version",
            MANIFEST.into(),
            b"return {game_id='test-game',api_version=1,version='2'}",
        ),
        ("syntax", MANIFEST.into(), b"return {{"),
        ("binary", MANIFEST.into(), b"\0return {}"),
        ("non UTF8", MANIFEST.into(), b"\xff"),
        ("malformed JSON", "{".into(), SCRIPT.as_bytes()),
        (
            "unknown field",
            MANIFEST.replace("{", "{\"sha256\":\"fake\","),
            SCRIPT.as_bytes(),
        ),
        (
            "duplicate field",
            MANIFEST.replace("{", "{\"game_id\":\"test-game\","),
            SCRIPT.as_bytes(),
        ),
    ] {
        reject(
            t.path(),
            label,
            &archive(&[("module.json", manifest.as_bytes()), ("main.luau", script)]),
        );
    }
}

#[test]
fn unsafe_extra_and_colliding_entries_never_create_state() {
    let t = tempfile::tempdir().expect("fixture operation");
    for name in [
        "../main.luau",
        "/main.luau",
        "C:/main.luau",
        "dir/main.luau",
        "dir\\main.luau",
        "payload.exe",
        "main.LUAU",
        "NUL.luau",
        "main.luau/",
        "main.luau.",
    ] {
        reject(
            t.path(),
            name,
            &archive(&[
                ("module.json", MANIFEST.as_bytes()),
                (name, SCRIPT.as_bytes()),
            ]),
        );
    }
    reject(
        t.path(),
        "extra file",
        &archive(&[
            ("module.json", MANIFEST.as_bytes()),
            ("main.luau", SCRIPT.as_bytes()),
            ("extra.luau", b"return {}"),
        ]),
    );
    let manifest = MANIFEST.replace("main.luau", "mainxx.luau");
    let valid = archive(&[
        ("module.json", manifest.as_bytes()),
        ("mainxx.luau", SCRIPT.as_bytes()),
    ]);
    for name in [b"module.json".as_slice(), b"MODULE.JSON".as_slice()] {
        let mut bytes = valid.clone();
        let offsets: Vec<_> = bytes
            .windows(11)
            .enumerate()
            .filter_map(|(i, b)| (b == b"mainxx.luau").then_some(i))
            .collect();
        for at in offsets {
            bytes[at..at + 11].copy_from_slice(name);
        }
        reject(t.path(), "duplicate/case-colliding record", &bytes);
    }
}

#[test]
fn zip_flags_links_metadata_sizes_crc_and_input_bounds_are_checked() {
    let t = tempfile::tempdir().expect("fixture operation");
    let valid = archive(&[
        ("module.json", MANIFEST.as_bytes()),
        ("main.luau", SCRIPT.as_bytes()),
    ]);
    let central = valid
        .windows(4)
        .position(|b| b == b"PK\x01\x02")
        .expect("fixture operation");
    for (label, offset, replacement) in [
        ("encryption", central + 8, vec![1, 0]),
        ("data descriptor", central + 8, vec![8, 0]),
        (
            "symlink",
            central + 38,
            (0o120644u32 << 16).to_le_bytes().to_vec(),
        ),
        (
            "executable",
            central + 38,
            (0o100755u32 << 16).to_le_bytes().to_vec(),
        ),
        ("reparse", central + 38, 0x400u32.to_le_bytes().to_vec()),
        ("extra metadata", central + 30, vec![0xff, 0xff]),
        ("comment", central + 32, vec![1, 0]),
        (
            "oversized entry",
            central + 24,
            (17 * 1024u32).to_le_bytes().to_vec(),
        ),
        (
            "ZIP64 offset",
            central + 42,
            u32::MAX.to_le_bytes().to_vec(),
        ),
        (
            "truncated sizes",
            central + 20,
            u32::MAX.to_le_bytes().to_vec(),
        ),
    ] {
        let mut bytes = valid.clone();
        bytes[offset..offset + replacement.len()].copy_from_slice(&replacement);
        reject(t.path(), label, &bytes);
    }
    let mut crc = valid.clone();
    let at = 30 + "module.json".len();
    crc[at] ^= 1;
    reject(t.path(), "CRC/content corruption", &crc);
    reject(t.path(), "input size", &vec![0; 288 * 1024 + 1]);
    reject(
        t.path(),
        "script limit",
        &archive(&[
            ("module.json", MANIFEST.as_bytes()),
            ("main.luau", &vec![b' '; 256 * 1024 + 1]),
        ]),
    );
    reject(t.path(), "missing end record", &valid[..valid.len() - 10]);
}
