use modn_core::game_modules::{check_module, check_module_with_roots};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn fixture() -> (TempDir, PathBuf, PathBuf) {
    let base = tempfile::tempdir().expect("temporary game");
    let module = base.path().join("external-module");
    let game = base.path().join("test-game");
    fs::create_dir(&module).expect("module");
    fs::create_dir(&game).expect("game");
    fs::write(
        module.join("module.json"),
        r#"{"game_id":"example-game","version":"1.0","api_version":1,"entrypoint":"main.lua"}"#,
    )
    .expect("manifest");
    fs::write(game.join("game.marker"), b"abc").expect("game marker");
    fs::create_dir(game.join("packages")).expect("packages");
    let file = fs::File::create(game.join("packages/test.zip")).expect("archive file");
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file(
        "content/example.txt",
        zip::write::SimpleFileOptions::default(),
    )
    .expect("archive entry");
    zip.write_all(b"hello").expect("content");
    zip.finish().expect("ZIP finish");
    (base, module, game)
}

fn run(script: &str, root: &Path, module: &Path) -> anyhow::Result<()> {
    fs::write(module.join("main.lua"), script).expect("replace module");
    check_module_with_roots(module, &[root.to_path_buf()]).map(|_| ())
}

fn failure(script: &str, root: &Path, module: &Path) -> String {
    format!(
        "{:#}",
        run(script, root, module).expect_err("read should fail")
    )
}

const SUCCESS: &str = "return { game_id='example-game', api_version=mods_host.api_version }";

#[test]
fn module_discovers_authorized_game_and_inspects_zip_and_hash() {
    let (_temp, module, game) = fixture();
    run(
        r#"
        local roots = mods_host.discovery.roots()
        assert(#roots == 1 and roots[1].id == 1)
        local metadata = mods_host.fs.metadata(roots[1].id, "game.marker")
        assert(metadata.kind == "file" and metadata.bytes == 3)
        assert(mods_host.fs.metadata(1, "packages").kind == "directory")
        local digest = mods_host.hash.sha256(1, "game.marker")
        assert(digest == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        local pkg = mods_host.archive.inspect(1, "packages/test.zip")
        assert(pkg.format == "zip" and pkg.metadata_only)
        assert(#pkg.files == 1 and pkg.files[1].entry == "content/example.txt")
        assert(pkg.files[1].bytes == 5 and pkg.total_uncompressed_bytes == 5)
        assert(io == nil and os == nil and print == nil)
        return { game_id='example-game', api_version=mods_host.api_version }
    "#,
        &game,
        &module,
    )
    .expect("read-only fixture loads external game");
    assert_eq!(
        fs::read(game.join("game.marker")).expect("marker intact"),
        b"abc"
    );
    assert!(game.join("packages/test.zip").exists());
}

#[test]
fn no_implicit_discovery_or_default_filesystem_authorization() {
    let (_temp, module, game) = fixture();
    fs::write(
        module.join("main.lua"),
        r#"
        assert(#mods_host.discovery.roots() == 0)
        assert(not pcall(function() return mods_host.fs.metadata(1, "game.marker") end))
        return { game_id='example-game', api_version=mods_host.api_version }
    "#,
    )
    .expect("script");
    check_module(&module).expect("unprivileged module loads without roots");
    run(
        "assert(#mods_host.discovery.roots() == 1); "
            .to_owned()
            .as_str(),
        &game,
        &module,
    )
    .expect_err("module without return is invalid");
}

#[test]
fn traversal_absolute_paths_and_other_roots_are_denied() {
    let (base, module, game) = fixture();
    fs::write(base.path().join("private.txt"), b"confidential").expect("private");
    for attempted in [
        "../private.txt",
        "../../private.txt",
        "/tmp/private.txt",
        "C:/Windows/win.ini",
        "packages/../game.marker",
        "./game.marker",
        "packages//test.zip",
        "packages\\test.zip",
        "game.marker:",
    ] {
        let script = format!(
            r#"
            local ok = pcall(function() return mods_host.fs.metadata(1, {attempted:?}) end)
            assert(not ok)
            return {{game_id='example-game',api_version=mods_host.api_version}}
        "#
        );
        run(&script, &game, &module).unwrap_or_else(|error| {
            panic!("unsafe path {attempted:?} unexpectedly processed: {error:#}")
        });
    }
    let error = failure(
        "mods_host.hash.sha256(1, '../private.txt'); return {}",
        &game,
        &module,
    );
    assert!(error.contains("unsafe relative path"), "{error}");
}

#[test]
fn invalid_root_ids_and_nonexistent_files_fail_explicitly() {
    let (_temp, module, game) = fixture();
    for (method, path) in [
        ("mods_host.fs.metadata", "game.marker"),
        ("mods_host.hash.sha256", "game.marker"),
        ("mods_host.archive.inspect", "packages/test.zip"),
    ] {
        let script = format!("{method}(2, {path:?}); {SUCCESS}");
        let error = failure(&script, &game, &module);
        assert!(
            error.contains("unknown authorized root identifier"),
            "{error}"
        );
    }
    let error = failure(
        "mods_host.fs.metadata(1, 'missing'); return {}",
        &game,
        &module,
    );
    assert!(error.contains("cannot inspect requested path"), "{error}");
}

#[test]
fn oversized_files_and_excessive_operations_are_rejected() {
    let (_temp, module, game) = fixture();
    fs::write(game.join("big.bin"), vec![0u8; 9 * 1024 * 1024]).expect("large file");
    fs::copy(game.join("big.bin"), game.join("big.zip")).expect("large archive fixture");
    let error = failure(
        "mods_host.hash.sha256(1,'big.bin'); return {}",
        &game,
        &module,
    );
    assert!(error.contains("per-call read limit"), "{error}");
    fs::write(game.join("chunk.bin"), vec![1u8; 5 * 1024 * 1024]).expect("chunk");
    let error = failure(
        r#"
        for i = 1, 4 do mods_host.hash.sha256(1, "chunk.bin") end
        return {}
    "#,
        &game,
        &module,
    );
    assert!(error.contains("byte budget exceeded"), "{error}");
    let error = failure(
        r#"
        for i = 1, 200 do mods_host.fs.metadata(1, "game.marker") end
        return {}
    "#,
        &game,
        &module,
    );
    assert!(error.contains("call budget exceeded"), "{error}");
    let error = failure(
        "mods_host.archive.inspect(1,'big.zip'); return {}",
        &game,
        &module,
    );
    assert!(error.contains("per-call read limit"), "{error}");
}

#[test]
fn encoded_7z_headers_are_never_passed_to_unbounded_native_parser() {
    let (_temp, module, game) = fixture();
    fs::write(game.join("packages/encoded.7z"), b"7z\xbc\xaf\x27\x1c").expect("7z fixture");
    let error = failure(
        "mods_host.archive.inspect(1, 'packages/encoded.7z'); return {}",
        &game,
        &module,
    );
    assert!(error.contains("ZIP/JAR only"), "{error}");
    fs::copy(
        game.join("packages/test.zip"),
        game.join("packages/test.jar"),
    )
    .expect("jar fixture");
    run(
        "assert(mods_host.archive.inspect(1, 'packages/test.jar').format == 'jar'); return {game_id='example-game',api_version=mods_host.api_version}",
        &game,
        &module,
    )
    .expect("bounded ZIP-formatted JAR still supported");
}

#[test]
fn malformed_or_unsafe_archive_is_rejected_through_host() {
    let (_temp, module, game) = fixture();
    let path = game.join("packages/unsafe.zip");
    let output = fs::File::create(&path).expect("fixture file");
    let mut writer = zip::ZipWriter::new(output);
    writer
        .start_file("../outside.txt", zip::write::SimpleFileOptions::default())
        .expect("unsafe entry fixture");
    writer.write_all(b"not allowed").expect("payload");
    writer.finish().expect("ZIP finish");
    let error = failure(
        "mods_host.archive.inspect(1,'packages/unsafe.zip'); return {}",
        &game,
        &module,
    );
    assert!(error.contains("unsafe archive entry"), "{error}");
}

#[test]
fn roots_require_explicit_real_directories_and_unique_identities() {
    let (base, module, game) = fixture();
    fs::write(module.join("main.lua"), SUCCESS).expect("script");
    let missing = base.path().join("missing-game");
    assert!(check_module_with_roots(&module, &[missing]).is_err());
    assert!(check_module_with_roots(&module, &[game.clone(), game.clone()]).is_err());
    assert!(check_module_with_roots(&module, &vec![game; 9]).is_err());
}

#[cfg(unix)]
#[test]
fn linked_components_and_root_symlinks_are_rejected() {
    use std::os::unix::fs::symlink;
    let (base, module, game) = fixture();
    let outside = base.path().join("outside-secret");
    fs::write(&outside, b"secret").expect("secret");
    symlink(&outside, game.join("linked")).expect("linked file");
    let error = failure(
        "mods_host.hash.sha256(1,'linked'); return {}",
        &game,
        &module,
    );
    assert!(error.contains("linked or special"), "{error}");
    let dir = base.path().join("elsewhere");
    fs::create_dir(&dir).expect("external");
    fs::write(dir.join("secret.txt"), "secret").expect("external file");
    symlink(&dir, game.join("linked-dir")).expect("linked directory");
    let error = failure(
        "mods_host.fs.metadata(1,'linked-dir/secret.txt'); return {}",
        &game,
        &module,
    );
    assert!(error.contains("linked or special"), "{error}");
    let alias = base.path().join("game-alias");
    symlink(&game, &alias).expect("root link");
    assert!(check_module_with_roots(&module, &[alias]).is_err());
}

#[cfg(windows)]
#[test]
fn junction_components_and_roots_are_rejected() {
    let (base, module, game) = fixture();
    let outside = base.path().join("outside");
    fs::create_dir(&outside).expect("outside");
    fs::write(outside.join("secret.txt"), b"secret").expect("secret");
    let link = game.join("linked-dir");
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&outside)
        .output()
        .expect("junction");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let error = failure(
        "mods_host.fs.metadata(1,'linked-dir/secret.txt'); return {}",
        &game,
        &module,
    );
    assert!(error.contains("linked or special"), "{error}");
    assert!(check_module_with_roots(&module, &[link]).is_err());
}

#[test]
fn bounded_directory_listing_and_utf8_text_reads_are_root_scoped() {
    let (_temp, module, game) = fixture();
    fs::write(game.join("oversized.txt"), vec![b'x'; 65 * 1024]).expect("large text fixture");
    run(
        r#"
        local listing = mods_host.directory.list(1, "packages")
        assert(#listing.items == 1 and not listing.incomplete)
        assert(listing.items[1].name == "test.zip")
        assert(listing.items[1].kind == "file")
        assert(mods_host.directory.read_text(1, "game.marker") == "abc")
        assert(not pcall(function()
            return mods_host.directory.read_text(1, "oversized.txt")
        end))
        assert(not pcall(function()
            return mods_host.directory.read_text(1, "../outside.txt")
        end))
        assert(not pcall(function()
            return mods_host.directory.list(1, "game.marker")
        end))
        assert(not pcall(function()
            return mods_host.directory.list(2, "packages")
        end))
        return {game_id="example-game", api_version=mods_host.api_version}
    "#,
        &game,
        &module,
    )
    .expect("read-only directory and UTF-8 metadata");
}
