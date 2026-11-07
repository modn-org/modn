#[path = "support/main_bucket.rs"]
mod main_bucket;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output};

fn pack(source: &Path, zip: &Path, extra: Option<(&str, &[u8])>) {
    fn files(root: &Path, current: &Path, output: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(current).expect("fixture directory") {
            let path = entry.expect("fixture entry").path();
            if path.is_dir() {
                files(root, &path, output);
            } else {
                output.push((
                    path.strip_prefix(root)
                        .expect("fixture relative path")
                        .to_str()
                        .expect("fixture UTF-8 path")
                        .replace('\\', "/"),
                    fs::read(path).expect("fixture file"),
                ));
            }
        }
    }
    let mut entries = Vec::new();
    files(source, source, &mut entries);
    if let Some((name, bytes)) = extra {
        entries.retain(|(existing, _)| existing != name);
        entries.push((name.into(), bytes.to_vec()));
    }
    entries.sort();
    let mut writer = zip::ZipWriter::new(fs::File::create(zip).expect("fixture ZIP"));
    for (name, bytes) in entries {
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .expect("fixture ZIP entry");
        writer.write_all(&bytes).expect("fixture ZIP bytes");
    }
    writer.finish().expect("fixture ZIP complete");
    sidecar(zip);
}

fn sidecar(zip: &Path) {
    let hash = format!(
        "{:x}",
        Sha256::digest(fs::read(zip).expect("fixture ZIP bytes"))
    );
    fs::write(
        format!("{}.sha256", zip.display()),
        format!("{hash} *main-bucket.zip\n"),
    )
    .expect("fixture checksum");
}

fn repository() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main")
}

fn run(data: &Path, source: &str, args: &[&str]) -> Output {
    Command::new(main_bucket::modn_binary())
        .env("LOCALAPPDATA", data)
        .env("MODN_MAIN_BUCKET_SOURCE", source)
        .env_remove("MODN_GAME_MODULES_DIR")
        .args(args)
        .output()
        .expect("run fixture command")
}

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("fixture command UTF-8 output")
}

#[test]
fn verified_update_installs_catalog_and_module_and_reports_change() {
    let data = tempfile::tempdir().unwrap();
    let zip = data.path().join("main.zip");
    pack(&repository(), &zip, None);
    let source = zip.to_str().unwrap();
    assert!(
        success(run(data.path(), source, &["--json", "bucket", "update"]))
            .contains("\"changed\":true")
    );
    assert!(success(run(data.path(), source, &["search"])).contains("red4ext"));
    let game = data.path().join("fake-game");
    fs::create_dir_all(game.join("bin/x64")).unwrap();
    fs::write(game.join("bin/x64/Cyberpunk2077.exe"), "fixture").unwrap();
    success(run(
        data.path(),
        source,
        &[
            "game",
            "add",
            "cyberpunk-2077",
            "--path",
            game.to_str().unwrap(),
        ],
    ));
    assert!(success(run(
        data.path(),
        source,
        &["--json", "bucket", "update", "main"]
    ))
    .contains("\"changed\":false"));
}

#[test]
fn rejected_updates_keep_previous_bucket_unchanged() {
    let data = tempfile::tempdir().unwrap();
    let zip = data.path().join("main.zip");
    pack(&repository(), &zip, None);
    let source = zip.to_str().unwrap();
    success(run(data.path(), source, &["bucket", "update"]));
    let main = data.path().join("modn/buckets/main");
    let original = modn_core::bucket_digest::directory_digest(&main).unwrap();
    let module = "games/cyberpunk-2077/module.json";
    let current = fs::read_to_string(repository().join(module)).unwrap();
    let future = current.replace("\"api_version\":1", "\"api_version\":3");
    assert_ne!(future, current, "fixture module declares host API 1");
    for failure in ["checksum", "traversal", "module", "api", "http", "name"] {
        pack(&repository(), &zip, None);
        match failure {
            "checksum" => fs::write(format!("{source}.sha256"), "0".repeat(64)).unwrap(),
            "traversal" => pack(&repository(), &zip, Some(("../escaped", b"hostile"))),
            "api" => pack(&repository(), &zip, Some((module, future.as_bytes()))),
            "module" => {
                let mut writer = zip::ZipWriter::new_append(
                    fs::OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open(&zip)
                        .unwrap(),
                )
                .unwrap();
                writer
                    .start_file(
                        "games/bad-game/module.json",
                        zip::write::SimpleFileOptions::default(),
                    )
                    .unwrap();
                writer
                    .write_all(
                        &fs::read(repository().join("games/cyberpunk-2077/module.json")).unwrap(),
                    )
                    .unwrap();
                writer
                    .start_file(
                        "games/bad-game/module.lua",
                        zip::write::SimpleFileOptions::default(),
                    )
                    .unwrap();
                writer.write_all(b"this is invalid Lua!").unwrap();
                writer.finish().unwrap();
                sidecar(&zip);
            }
            _ => {}
        }
        let output = run(
            data.path(),
            if failure == "http" {
                "http://example.invalid/main.zip"
            } else {
                source
            },
            if failure == "name" {
                &["bucket", "update", "community"]
            } else {
                &["bucket", "update"]
            },
        );
        assert!(!output.status.success(), "{failure}");
        if failure == "api" {
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("incompatible host API"),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        assert_eq!(
            modn_core::bucket_digest::directory_digest(&main).unwrap(),
            original,
            "{failure}"
        );
        assert!(!data.path().join("modn/buckets/.main-staging").exists());
        assert!(!data.path().join("modn/buckets/.main-aside").exists());
        assert!(!data.path().join("modn/buckets/escaped").exists());
    }
}

#[test]
fn interrupted_swaps_restore_or_clean_before_failed_update() {
    let data = tempfile::tempdir().unwrap();
    let zip = data.path().join("main.zip");
    pack(&repository(), &zip, None);
    let source = zip.to_str().unwrap();
    success(run(data.path(), source, &["bucket", "update"]));
    let root = data.path().join("modn/buckets");
    let digest = modn_core::bucket_digest::directory_digest(&root.join("main")).unwrap();
    for main_present in [false, true] {
        fs::rename(root.join("main"), root.join(".main-aside")).unwrap();
        if main_present {
            fs::create_dir(root.join("main")).unwrap();
            fs::write(root.join("main/marker"), "committed new tree").unwrap();
        }
        fs::create_dir(root.join(".main-staging")).unwrap();
        fs::write(root.join(".main-staging/partial"), "partial").unwrap();
        fs::write(format!("{source}.sha256"), "0".repeat(64)).unwrap();
        assert!(!run(data.path(), source, &["bucket", "update"])
            .status
            .success());
        assert!(!root.join(".main-staging").exists());
        assert!(!root.join(".main-aside").exists());
        if main_present {
            assert_eq!(
                fs::read_to_string(root.join("main/marker")).unwrap(),
                "committed new tree"
            );
        } else {
            assert_eq!(
                modn_core::bucket_digest::directory_digest(&root.join("main")).unwrap(),
                digest
            );
        }
    }
    fs::rename(root.join("main"), root.join(".main-aside")).unwrap();
    fs::remove_file(format!("{source}.sha256")).unwrap();
    assert!(!run(data.path(), source, &["bucket", "update"])
        .status
        .success());
    assert!(root.join("main/marker").is_file());
    assert!(!root.join(".main-aside").exists());
}
