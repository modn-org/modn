#[path = "support/main_bucket.rs"]
mod main_bucket;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
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
}

fn remove_signature(zip: &Path) {
    match fs::remove_file(format!("{}.minisig", zip.display())) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("cannot remove fixture signature: {error}"),
    }
}

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main")
}

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("fixture tree");
    for entry in fs::read_dir(source).expect("fixture entries") {
        let entry = entry.expect("fixture entry");
        if entry.file_name() == ".git" {
            continue;
        }
        let target = target.join(entry.file_name());
        if entry.file_type().expect("fixture entry type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("fixture tree file");
        }
    }
}

fn fixture_tree() -> tempfile::TempDir {
    let tree = tempfile::tempdir().expect("fixture tree directory");
    copy_tree(&repository(), tree.path());
    tree
}

fn test_key_id() -> String {
    modn_core::buckets::CatalogPublicKey::parse(&main_bucket::test_public_key())
        .expect("fixture public key")
        .key_id()
        .to_string()
}

fn run(data: &Path, source: &str, args: &[&str]) -> Output {
    Command::new(main_bucket::modn_binary())
        .env("LOCALAPPDATA", data)
        .env("MODN_MAIN_BUCKET_SOURCE", source)
        .env(
            "MODN_TEST_MAIN_BUCKET_PUBLIC_KEY",
            main_bucket::test_public_key(),
        )
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

fn failure(output: &Output, expected: &str) {
    assert!(!output.status.success(), "{expected}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(expected), "{expected}: {stderr}");
}

#[test]
fn verified_update_installs_catalog_and_module_and_reports_change() {
    let data = tempfile::tempdir().unwrap();
    let zip = data.path().join("main.zip");
    let tree = fixture_tree();
    pack(tree.path(), &zip, None);
    main_bucket::write_signature(tree.path(), &zip);
    let source = zip.to_str().unwrap();
    let key_id = test_key_id();
    let output = success(run(data.path(), source, &["bucket", "update"]));
    assert!(
        output.contains(&format!("Verified main bucket signed by {key_id}.")),
        "{output}"
    );
    assert!(
        success(run(data.path(), source, &["--json", "bucket", "update"]))
            .contains("\"changed\":false")
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
    let tree = fixture_tree();
    pack(tree.path(), &zip, None);
    main_bucket::write_signature(tree.path(), &zip);
    let source = zip.to_str().unwrap();
    success(run(data.path(), source, &["bucket", "update"]));
    let main = data.path().join("modn/buckets/main");
    let original = modn_core::bucket_digest::directory_digest(&main).unwrap();
    let module = "games/cyberpunk-2077/module.json";
    let current = fs::read_to_string(repository().join(module)).unwrap();
    let future = current.replace("\"api_version\":1", "\"api_version\":3");
    assert_ne!(future, current, "fixture module declares host API 1");

    let cases = [
        ("unsigned", "is unsigned; Modn refuses unsigned releases"),
        ("sidecar", "is unsigned; Modn refuses unsigned releases"),
        ("malformed", "malformed catalog signature"),
        ("wrong-key", "does not match public key"),
        ("other-tree", "catalog signature verification failed"),
        ("tree-name", "expected \"main\""),
        ("traversal", "unsafe archive entry"),
        ("module", "failed validation"),
        ("api", "incompatible host API"),
        ("http", "must be an HTTPS URL"),
        ("name", "only main can be updated"),
    ];
    for (case, expected) in cases {
        remove_signature(&zip);
        match case {
            "unsigned" => {
                pack(tree.path(), &zip, None);
                fs::write(format!("{source}.sha256"), "0".repeat(64)).unwrap();
            }
            "sidecar" => pack(tree.path(), &zip, None),
            "malformed" => {
                pack(tree.path(), &zip, None);
                fs::write(format!("{source}.minisig"), "not a signature").unwrap();
            }
            "wrong-key" => {
                pack(tree.path(), &zip, None);
                main_bucket::write_foreign_signature(tree.path(), &zip);
            }
            "other-tree" => {
                let modified = fixture_tree();
                fs::write(modified.path().join(module), &future).unwrap();
                pack(modified.path(), &zip, None);
                main_bucket::write_signature(tree.path(), &zip);
            }
            "tree-name" => {
                let renamed = fixture_tree();
                let metadata = fs::read_to_string(renamed.path().join("bucket.json")).unwrap();
                fs::write(
                    renamed.path().join("bucket.json"),
                    metadata.replace("\"main\"", "\"community\""),
                )
                .unwrap();
                pack(renamed.path(), &zip, None);
                main_bucket::write_signature(renamed.path(), &zip);
            }
            "traversal" => {
                pack(tree.path(), &zip, Some(("../escaped", b"hostile")));
                main_bucket::write_signature(tree.path(), &zip);
            }
            "module" => {
                let modified = fixture_tree();
                let game = modified.path().join("games/bad-game");
                fs::create_dir_all(&game).unwrap();
                fs::copy(
                    tree.path().join("games/cyberpunk-2077/module.json"),
                    game.join("module.json"),
                )
                .unwrap();
                fs::write(game.join("module.lua"), "this is invalid Lua!").unwrap();
                pack(modified.path(), &zip, None);
                main_bucket::write_signature(modified.path(), &zip);
            }
            "api" => {
                let modified = fixture_tree();
                fs::write(modified.path().join(module), &future).unwrap();
                pack(modified.path(), &zip, None);
                main_bucket::write_signature(modified.path(), &zip);
            }
            "http" => remove_signature(&zip),
            "name" => {}
            _ => unreachable!(),
        }
        let output = run(
            data.path(),
            if case == "http" {
                "http://example.invalid/main.zip"
            } else {
                source
            },
            if case == "name" {
                &["bucket", "update", "community"]
            } else {
                &["bucket", "update"]
            },
        );
        failure(&output, expected);
        assert_eq!(
            modn_core::bucket_digest::directory_digest(&main).unwrap(),
            original,
            "{case}"
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
    let tree = fixture_tree();
    pack(tree.path(), &zip, None);
    main_bucket::write_signature(tree.path(), &zip);
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
        remove_signature(&zip);
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
    remove_signature(&zip);
    assert!(!run(data.path(), source, &["bucket", "update"])
        .status
        .success());
    assert!(root.join("main/marker").is_file());
    assert!(!root.join(".main-aside").exists());
}

#[test]
fn embedded_signer_key_refuses_a_release_signed_by_another_key() {
    let data = tempfile::tempdir().unwrap();
    let zip = data.path().join("main.zip");
    let tree = fixture_tree();
    pack(tree.path(), &zip, None);
    main_bucket::write_signature(tree.path(), &zip);
    let output = Command::new(main_bucket::modn_binary())
        .env("LOCALAPPDATA", data.path())
        .env("MODN_MAIN_BUCKET_SOURCE", zip.to_str().unwrap())
        .env_remove("MODN_TEST_MAIN_BUCKET_PUBLIC_KEY")
        .env_remove("MODN_GAME_MODULES_DIR")
        .args(["bucket", "update"])
        .output()
        .expect("run fixture command");
    failure(&output, "does not match public key 0E250AD52A3BA2AA");
    assert!(!data.path().join("modn/buckets/main").exists());
}

#[test]
fn sidecar_only_release_is_refused() {
    let data = tempfile::tempdir().unwrap();
    let zip = data.path().join("main.zip");
    pack(&repository(), &zip, None);
    fs::write(format!("{}.sha256", zip.display()), "0".repeat(64)).unwrap();
    let output = run(data.path(), zip.to_str().unwrap(), &["bucket", "update"]);
    failure(&output, "is unsigned; Modn refuses unsigned releases");
    assert!(!data.path().join("modn/buckets/main").exists());
}
