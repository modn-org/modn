use modn_core::game_modules::check_module;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn fixture() -> (TempDir, PathBuf) {
    let root = tempfile::tempdir().expect("create fixture");
    let directory = root.path().join("integration");
    fs::create_dir(&directory).expect("create module directory");
    write_manifest(&directory, "fixture-game", 1, "main.lua");
    write_script(
        &directory,
        "return { game_id = 'fixture-game', api_version = mods_host.api_version }",
    );
    (root, directory)
}

fn write_manifest(directory: &Path, game_id: &str, api_version: u32, entrypoint: &str) {
    fs::write(
        directory.join("module.json"),
        serde_json::json!({
            "game_id": game_id,
            "version": "1.2.3",
            "api_version": api_version,
            "entrypoint": entrypoint
        })
        .to_string(),
    )
    .expect("write module manifest");
}

fn write_script(directory: &Path, script: &str) {
    fs::write(directory.join("main.lua"), script).expect("write module script");
}

#[test]
fn local_fixture_is_loaded_without_host_rebuild() {
    let (_root, directory) = fixture();
    let first = check_module(&directory).expect("initial module loads");
    assert_eq!(first.game_id, "fixture-game");
    assert_eq!(first.version, "1.2.3");
    assert_eq!(first.api_version, 1);

    write_script(
        &directory,
        "return {game_id='other-game', api_version=mods_host.api_version}",
    );
    assert!(format!("{:#}", check_module(&directory).unwrap_err()).contains("identity"));
    write_manifest(&directory, "other-game", 1, "main.lua");
    let updated = check_module(&directory).expect("updated module loads without recompile");
    assert_eq!(updated.game_id, "other-game");
}

#[test]
fn standard_lua54_syntax_loads_and_luau_assignment_is_rejected() {
    let (_root, directory) = fixture();
    write_script(
        &directory,
        "local flags = 7 & 3; local ratio = 7 // 2; assert(flags + ratio == 6); return {game_id='fixture-game', api_version=mods_host.api_version}",
    );
    check_module(&directory).expect("Lua 5.4 bitwise and floor division are supported");
    write_script(
        &directory,
        "local count = 0; count += 1; return {game_id='fixture-game', api_version=mods_host.api_version}",
    );
    assert!(
        check_module(&directory).is_err(),
        "Luau compound assignment must not load"
    );
}

#[test]
fn incompatible_manifest_or_script_fails_closed() {
    let (_root, directory) = fixture();
    write_manifest(&directory, "fixture-game", 3, "main.lua");
    assert!(
        format!("{:#}", check_module(&directory).unwrap_err()).contains("incompatible host API")
    );
    write_manifest(&directory, "fixture-game", 1, "main.lua");
    write_script(
        &directory,
        "return {game_id='fixture-game', api_version=999}",
    );
    assert!(format!("{:#}", check_module(&directory).unwrap_err()).contains("identity"));
    write_script(&directory, "error('intentional failure')");
    assert!(format!("{:#}", check_module(&directory).unwrap_err())
        .contains("Lua module execution failed"));
    write_script(&directory, "return 2");
    assert!(check_module(&directory).is_err());
}

#[test]
fn no_os_or_file_write_capability_is_exposed() {
    let (_root, directory) = fixture();
    let target = directory.join("unauthorized.txt");
    write_script(
        &directory,
        r#"
        assert(io == nil and os == nil and package == nil)
        assert(require == nil and loadfile == nil and dofile == nil)
        assert(load == nil and loadstring == nil and debug == nil)
        assert(print == nil and warn == nil)
        assert(type(mods_host.api_version) == "number")
        assert(not pcall(function() mods_host.api_version = 99 end))
        return { game_id='fixture-game', api_version=mods_host.api_version }
    "#,
    );
    check_module(&directory).expect("restricted globals remain inaccessible");
    write_script(
        &directory,
        r#"
        local file = io.open("unauthorized.txt", "w")
        file:write("should not appear")
        return {game_id='fixture-game', api_version=mods_host.api_version}
    "#,
    );
    assert!(check_module(&directory).is_err());
    assert!(!target.exists());
}

#[test]
fn loops_and_memory_exhaustion_return_explicit_errors() {
    let (_root, directory) = fixture();
    write_script(&directory, "while true do end");
    let error = format!("{:#}", check_module(&directory).unwrap_err());
    assert!(error.contains("budget exceeded"), "{error}");
    write_script(&directory, "local x = string.rep('x', 16000000); return {}");
    assert!(check_module(&directory).is_err());
}

#[test]
fn malformed_manifests_and_untrusted_entrypoints_are_rejected() {
    let (_root, directory) = fixture();
    write_manifest(&directory, "fixture-game", 1, "../outside.lua");
    assert!(format!("{:#}", check_module(&directory).unwrap_err()).contains("entrypoint"));
    write_manifest(&directory, "fixture-game", 1, "subdir/main.lua");
    assert!(check_module(&directory).is_err());
    write_manifest(&directory, "Invalid Game", 1, "main.lua");
    assert!(check_module(&directory).is_err());
    fs::write(directory.join("module.json"), "{broken").expect("overwrite manifest");
    assert!(format!("{:#}", check_module(&directory).unwrap_err()).contains("invalid module.json"));
}

#[test]
fn bytecode_or_non_utf8_entrypoints_are_rejected_as_source() {
    let (_root, directory) = fixture();
    fs::write(directory.join("main.lua"), [0xff, 0, 0]).expect("binary fixture");
    let error = format!("{:#}", check_module(&directory).unwrap_err());
    assert!(error.contains("UTF-8 source"), "{error}");
    fs::write(directory.join("main.lua"), b"return {}\0").expect("NUL fixture");
    let error = format!("{:#}", check_module(&directory).unwrap_err());
    assert!(error.contains("binary chunks"), "{error}");
}

#[test]
fn oversized_manifest_and_script_are_rejected() {
    let (_root, directory) = fixture();
    write_script(&directory, &" ".repeat(256 * 1024 + 1));
    assert!(format!("{:#}", check_module(&directory).unwrap_err()).contains("no larger"));
    write_script(&directory, "return {game_id='fixture-game',api_version=1}");
    fs::write(directory.join("module.json"), " ".repeat(16 * 1024 + 1))
        .expect("write large manifest");
    assert!(check_module(&directory).is_err());
}

#[cfg(unix)]
#[test]
fn linked_script_and_linked_module_directory_are_rejected() {
    use std::os::unix::fs::symlink;
    let (root, directory) = fixture();
    let external = root.path().join("outside.lua");
    fs::write(&external, "return {}").expect("write outside fixture");
    fs::remove_file(directory.join("main.lua")).expect("remove script");
    symlink(&external, directory.join("main.lua")).expect("link script");
    assert!(check_module(&directory).is_err());
    let alias = root.path().join("module-alias");
    symlink(&directory, &alias).expect("link module");
    assert!(check_module(&alias).is_err());
}

#[cfg(windows)]
#[test]
fn linked_script_directory_junction_is_rejected() {
    let (root, directory) = fixture();
    let alias = root.path().join("module-junction");
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&alias)
        .arg(&directory)
        .output()
        .expect("create junction");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(check_module(&alias).is_err());
}
