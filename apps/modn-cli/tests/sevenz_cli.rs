//! End-to-end 7z execution, including a Browser Extension redscript layout.
use serde_json::Value;
use sevenz_rust2::{ArchiveEntry, ArchiveWriter, SourceReader};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::{tempdir, TempDir};

fn fixture() -> (TempDir, PathBuf, PathBuf) {
    let temp = tempdir().expect("test fixture");
    let game = temp.path().join("game");
    fs::create_dir_all(game.join("bin/x64")).expect("game directory");
    fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").expect("game marker");
    let compiler = game.join("engine/tools/scc.exe");
    fs::create_dir_all(compiler.parent().expect("parent")).expect("tool directory");
    fs::write(compiler, b"compiler marker").expect("compiler fixture");
    let state = temp.path().join("state");
    (temp, game, state)
}

fn sevenz(path: &Path, files: &[(&str, &[u8])]) {
    let mut writer = ArchiveWriter::create(path).expect("7z writer");
    for &(name, content) in files {
        writer
            .push_archive_entry(ArchiveEntry::new_file(name), Some(Cursor::new(content)))
            .expect("7z member");
    }
    writer.finish().expect("complete 7z");
}

fn cli(state: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_modn"))
        .args(["--game", "cyberpunk-2077", "--json", "--state-dir"])
        .arg(state)
        .args(args)
        .output()
        .expect("modn executable")
}

fn success(state: &Path, args: &[&str]) -> Value {
    let run = cli(state, args);
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    serde_json::from_slice(&run.stdout).expect("CLI JSON")
}

fn preview(game: &Path, state: &Path, archive: &Path) -> Value {
    success(
        state,
        &[
            "install",
            archive.to_str().expect("7z"),
            "--dry-run",
            "--path",
            game.to_str().expect("game"),
        ],
    )
}

const BROWSER: &[(&str, &[u8])] = &[
    (
        "r6/scripts/BrowserExtension/browserController.overrides.reds",
        b"script a",
    ),
    (
        "r6/scripts/BrowserExtension/BrowserExtension.Classes.reds",
        b"script b",
    ),
    (
        "r6/scripts/BrowserExtension/BrowserExtension.Config.reds",
        b"script c",
    ),
    (
        "r6/scripts/BrowserExtension/BrowserExtension.DataStructures.reds",
        b"script d",
    ),
    (
        "r6/scripts/BrowserExtension/BrowserExtension.System.reds",
        b"script e",
    ),
    (
        "r6/scripts/BrowserExtension/HomePagePagination.reds",
        b"script f",
    ),
    (
        "r6/scripts/BrowserExtension/ModSettingsConditional.reds",
        b"script g",
    ),
];

#[test]
fn browser_extension_7z_installs_uninstalls_and_preserves_unmanaged_files() {
    let (temp, game, state) = fixture();
    let archive = temp.path().join("browser.7z");
    sevenz(&archive, BROWSER);
    let root = game.to_str().expect("game");
    let source = archive.to_str().expect("source");
    let unmanaged = game.join("r6/scripts/BrowserExtension/user-notes.json");
    fs::create_dir_all(unmanaged.parent().expect("shared dir")).expect("shared path");
    fs::write(&unmanaged, b"user data").expect("unmanaged");
    let plan = preview(&game, &state, &archive);
    assert_eq!(plan["status"], "reviewable", "{plan}");
    assert_eq!(plan["operations"].as_array().expect("operations").len(), 7);
    assert!(!state.exists(), "preview created manager state");
    for &(path, _) in BROWSER {
        assert!(!game.join(path).exists(), "preview wrote into game");
    }
    let installed = success(&state, &["install", source, "--yes", "--path", root]);
    let id = installed["id"].as_str().expect("package ID");
    assert_eq!(installed["files"].as_array().expect("files").len(), 7);
    assert!(installed["directories"]
        .as_array()
        .expect("directories")
        .is_empty());
    for &(path, bytes) in BROWSER {
        assert_eq!(fs::read(game.join(path)).expect("payload"), bytes, "{path}");
    }
    assert!(
        !cli(&state, &["install", source, "--yes", "--path", root])
            .status
            .success(),
        "duplicate installation must reject owned files"
    );
    let modified = game.join(BROWSER[0].0);
    fs::write(&modified, b"user edit").expect("edit");
    assert!(
        !cli(&state, &["uninstall", id, "--yes", "--path", root])
            .status
            .success(),
        "modified managed file must block uninstall"
    );
    fs::write(&modified, BROWSER[0].1).expect("restore");
    success(&state, &["uninstall", id, "--yes", "--path", root]);
    for &(path, _) in BROWSER {
        assert!(!game.join(path).exists(), "managed member survived: {path}");
    }
    assert_eq!(
        fs::read(&unmanaged).expect("unmanaged file preserved"),
        b"user data"
    );
    assert!(success(&state, &["list", "--path", root])
        .as_array()
        .expect("ledger")
        .is_empty());
}

#[test]
fn sevenz_rejects_unsafe_payloads_and_existing_files() {
    let (temp, game, state) = fixture();
    let archive = temp.path().join("untrusted.7z");
    let root = game.to_str().expect("game");
    let name = "r6/scripts/BrowserExtension/Valid.reds";
    for bad in [
        "r6/scripts/BrowserExtension/autorun.cmd",
        "r6/scripts/BrowserExtension/unrecognized.dll",
        "unrecognized/vendor/private.bin",
    ] {
        sevenz(&archive, &[(name, b"safe"), (bad, b"unsafe")]);
        let report = preview(&game, &state, &archive);
        assert_eq!(report["status"], "blocked", "{bad}: {report}");

        assert!(!cli(
            &state,
            &[
                "install",
                archive.to_str().expect("7z"),
                "--yes",
                "--path",
                root
            ]
        )
        .status
        .success());
        assert!(
            !game.join(name).exists(),
            "blocked archive wrote game files"
        );
    }
    sevenz(&archive, &[(name, b"manager copy")]);
    let target = game.join(name);
    fs::create_dir_all(target.parent().expect("parent")).expect("directory");
    fs::write(&target, b"manual content").expect("manual file");
    let report = preview(&game, &state, &archive);
    assert_eq!(report["status"], "blocked", "{report}");
    assert!(!cli(
        &state,
        &[
            "install",
            archive.to_str().expect("7z"),
            "--yes",
            "--path",
            root
        ]
    )
    .status
    .success());
    assert_eq!(fs::read(&target).expect("user file"), b"manual content");
    assert!(success(&state, &["list", "--path", root])
        .as_array()
        .expect("ledger")
        .is_empty());
}

#[test]
fn sevenz_solid_with_unselected_readme_keeps_selected_streams_aligned() {
    let (temp, game, state) = fixture();
    let archive = temp.path().join("solid.7z");
    let selected = "r6/scripts/BrowserExtension/System.reds";
    let entries: [(&str, &[u8]); 3] = [
        ("README.md", b"long unrelated document before scripts"),
        (selected, b"correct content after skipped document"),
        ("CHANGELOG.txt", b"trailing documentation"),
    ];
    let mut writer = ArchiveWriter::create(&archive).expect("archive writer");
    writer
        .push_archive_entries(
            entries
                .iter()
                .map(|(path, _)| ArchiveEntry::new_file(path))
                .collect(),
            entries
                .iter()
                .map(|(_, bytes)| SourceReader::new(Cursor::new(*bytes)))
                .collect(),
        )
        .expect("solid 7z block");
    writer.finish().expect("archive complete");
    let report = preview(&game, &state, &archive);
    assert_eq!(report["status"], "reviewable", "{report}");
    assert_eq!(
        report["operations"].as_array().expect("operations").len(),
        1
    );
    let root = game.to_str().expect("game");
    let installed = success(
        &state,
        &[
            "install",
            archive.to_str().expect("7z"),
            "--yes",
            "--path",
            root,
        ],
    );
    assert_eq!(
        fs::read(game.join(selected)).expect("selected"),
        entries[1].1
    );
    assert!(!game.join("README.md").exists());
    assert!(!game.join("CHANGELOG.txt").exists());
    success(
        &state,
        &[
            "uninstall",
            installed["id"].as_str().expect("id"),
            "--yes",
            "--path",
            root,
        ],
    );
    assert!(!game.join(selected).exists());
}

#[test]
fn damaged_7z_stream_does_not_publish_game_files() {
    let (temp, game, state) = fixture();
    let archive = temp.path().join("damaged.7z");
    let path = "r6/scripts/BrowserExtension/Version.reds";
    sevenz(
        &archive,
        &[(
            path,
            b"lots of uncompressed valid script content content content",
        )],
    );
    let mut bytes = fs::read(&archive).expect("7z bytes");
    assert!(bytes.len() > 50, "valid 7z header and payload");
    // First packed data byte begins immediately after the 32-byte 7z signature
    // header. Alter the compressed body, not the parseable metadata headers.
    bytes[34] ^= 0x40;
    fs::write(&archive, bytes).expect("tamper compressed stream");
    let report = preview(&game, &state, &archive);
    assert_eq!(report["status"], "reviewable", "{report}");
    let root = game.to_str().expect("game");
    assert!(!cli(
        &state,
        &[
            "install",
            archive.to_str().expect("7z"),
            "--yes",
            "--path",
            root
        ]
    )
    .status
    .success());
    assert!(
        !game.join(path).exists(),
        "corrupted 7z stream published content"
    );
    assert!(success(&state, &["list", "--path", root])
        .as_array()
        .expect("ledger")
        .is_empty());
}
