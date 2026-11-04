use super::*;

#[test]
fn invalid_development_source_cannot_redirect_bucket_registry_into_game() {
    let temp = TempDir::new().expect("fixture");
    let bucket = temp.path().join("community");
    seed(&bucket, &valid_manifest());
    let game = temp.path().join("game");
    let marker = game.join("bin/x64/Cyberpunk2077.exe");
    fs::create_dir_all(marker.parent().expect("parent")).expect("fixture");
    fs::write(&marker, b"unchanged").expect("marker");
    let invalid = temp.path().join("missing-module");
    let mismatched = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/test-game");

    let broken = temp.path().join("broken-policy");
    fs::create_dir(&broken).expect("fixture");
    fs::write(
        broken.join("module.json"),
        r#"{"game_id":"cyberpunk-2077","version":"1.0","api_version":1,"entrypoint":"main.lua"}"#,
    )
    .expect("manifest");
    fs::write(
        broken.join("main.lua"),
        "return { game_id='cyberpunk-2077', api_version=1, validate=function() error('broken validation') end }",
    )
    .expect("Lua");
    for source in [&invalid, &mismatched, &broken] {
        let result = main_bucket::command(env!("CARGO_BIN_EXE_modn"))
            .args(["--state-dir"])
            .arg(&game)
            .arg("--src")
            .arg(source)
            .args(["bucket", "add", "community"])
            .arg(&bucket)
            .output()
            .expect("invoke bucket add");
        assert!(
            !result.status.success(),
            "must reject source: {}",
            source.display()
        );
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("module")
                || String::from_utf8_lossy(&result.stderr).contains("broken validation"),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!game.join("buckets").exists());
        assert!(!game.join("state.sqlite").exists());
        assert_eq!(fs::read(&marker).expect("marker"), b"unchanged");
    }
    let normal = run(
        &game,
        &["add", "community", bucket.to_str().expect("bucket")],
    );
    assert!(!normal.status.success());
    assert!(output(&normal).contains("outside a game installation"));
    assert!(!game.join("buckets").exists());
}
