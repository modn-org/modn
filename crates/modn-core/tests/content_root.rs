use modn_core::game_modules::{check_module, with_module_source};
use modn_core::planning::{
    preview_adoption_with_appdata, preview_install_zip_with_appdata, preview_update_with_appdata,
};
use modn_core::state::Store;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;

const GAME: &str = "content-game";

fn module(root: &Path, manifest: &str, script: &str) -> PathBuf {
    let directory = root.join("module");
    fs::create_dir_all(&directory).expect("module directory");
    fs::write(directory.join("module.json"), manifest).expect("manifest");
    fs::write(directory.join("module.lua"), script).expect("script");
    directory
}

fn manifest(api: u32, content_root: &str) -> String {
    format!(
        r#"{{"game_id":"{GAME}","version":"1.0.0","api_version":{api},"entrypoint":"module.lua"{content_root}}}"#
    )
}

const SCRIPT: &str = r#"
return {
    game_id = "content-game", api_version = 2, version = "1.0.0",
    marker = "ContentGame.exe", common_paths = {},
    validate = function() return true end,
    steam_name = function() return "Content Game" end,
    is_epic = function() return false end,
    scan = function() return { mods = {}, framework_signals = {}, incomplete = false, redmod_available = false } end,
    doctor = function() return {} end,
    plan = function(files)
        local operations = {}
        for _, file in ipairs(files) do
            table.insert(operations, {
                source = file.source, destination = file.source,
                kind = "content-mod", package_root = "", root = "content",
            })
        end
        return { schema_version = 1, operations = operations, blockers = {} }
    end,
}
"#;

fn archive(path: &Path, files: &[(&str, &[u8])]) {
    let mut writer = zip::ZipWriter::new(File::create(path).expect("create ZIP"));
    for (name, bytes) in files {
        writer
            .start_file(*name, SimpleFileOptions::default())
            .expect("ZIP entry");
        writer.write_all(bytes).expect("write payload");
    }
    writer.finish().expect("finish ZIP");
}

fn setup() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().expect("temp");
    let game = temp.path().join("game");
    let appdata = temp.path().join("appdata");
    fs::create_dir_all(&game).expect("game");
    fs::write(game.join("ContentGame.exe"), b"marker").expect("marker");
    fs::create_dir_all(&appdata).expect("appdata");
    let module = module(
        temp.path(),
        &manifest(
            2,
            r#","content_root":{"base":"appdata","path":"Fixture/Mods"}"#,
        ),
        SCRIPT,
    );
    (temp, game, appdata, module)
}

#[test]
fn valid_manifest_loads_and_invalid_forms_are_rejected() {
    let (temp, _game, _appdata, module_dir) = setup();
    assert_eq!(
        check_module(&module_dir).expect("API 2 module").api_version,
        2
    );
    for (name, body) in [
        (
            "api1",
            manifest(
                1,
                r#","content_root":{"base":"appdata","path":"Fixture/Mods"}"#,
            ),
        ),
        (
            "base",
            manifest(
                2,
                r#","content_root":{"base":"localappdata","path":"Fixture/Mods"}"#,
            ),
        ),
        (
            "absolute",
            manifest(
                2,
                r#","content_root":{"base":"appdata","path":"C:/Fixture"}"#,
            ),
        ),
        (
            "parent",
            manifest(
                2,
                r#","content_root":{"base":"appdata","path":"Fixture/../Mods"}"#,
            ),
        ),
        (
            "reserved",
            manifest(2, r#","content_root":{"base":"appdata","path":"CON/Mods"}"#),
        ),
    ] {
        let directory = module(temp.path(), &body, SCRIPT);
        assert!(
            check_module(&directory).is_err(),
            "{name} content_root must be rejected"
        );
    }
}

#[test]
fn content_root_lifecycle_keeps_ownership_on_the_content_root() {
    let (temp, game, appdata, module) = setup();
    let first = temp.path().join("first.zip");
    archive(&first, &[("core.lua", b"version one")]);
    let state = temp.path().join("state/state.sqlite");
    let mut store = Store::open(&state).expect("state");

    with_module_source(Some(&module), || {
        let plan = preview_install_zip_with_appdata(GAME, &game, &first, &appdata).expect("plan");
        assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
        assert_eq!(plan.operations[0].root, "content");
        assert_eq!(
            plan.content_root.as_deref(),
            Some(
                appdata
                    .canonicalize()
                    .unwrap()
                    .join("Fixture")
                    .join("Mods")
                    .as_path()
            )
        );
        assert!(!game.join("core.lua").exists());
        assert!(
            !appdata.join("Fixture").exists(),
            "dry run must not create the content root"
        );

        let installed = store
            .install_operations_zip(&game, &first, &plan)
            .expect("install");
        assert_eq!(installed.files[0].root, "content");
        assert_eq!(
            fs::read(appdata.join("Fixture/Mods/core.lua")).expect("content file"),
            b"version one"
        );
        assert!(!game.join("core.lua").exists());
        let reloaded = store.installed(&game).expect("installed");
        assert_eq!(reloaded[0].files[0].root, "content");

        let second = temp.path().join("second.zip");
        archive(&second, &[("core.lua", b"version two")]);
        let update =
            preview_update_with_appdata(GAME, &game, &second, &[], &["core.lua".into()], &appdata)
                .expect("update preview");
        assert!(update.blockers.is_empty(), "{:?}", update.blockers);

        store.uninstall(&game, &installed.id).expect("uninstall");
        assert!(!appdata.join("Fixture/Mods/core.lua").exists());
        assert!(!game.join("core.lua").exists());
    });
}

#[test]
fn adoption_of_existing_content_files_is_root_qualified() {
    let (temp, game, appdata, module) = setup();
    let root = appdata.join("Fixture/Mods");
    fs::create_dir_all(&root).expect("content root");
    fs::write(root.join("core.lua"), b"manual").expect("manual file");
    let archive_path = temp.path().join("adopt.zip");
    archive(&archive_path, &[("core.lua", b"manual")]);
    let mut store = Store::open(&temp.path().join("state/state.sqlite")).expect("state");

    with_module_source(Some(&module), || {
        let preview =
            preview_adoption_with_appdata(GAME, &game, &archive_path, &appdata).expect("preview");
        assert!(preview.blockers.is_empty(), "{:?}", preview.blockers);
        let adopted = store
            .adopt_archive_with_appdata(
                GAME,
                &game,
                &archive_path,
                &preview.archive_blake3,
                modn_core::transactions::AdoptionApproval::Acknowledged,
                &appdata,
            )
            .expect("adopt");
        assert_eq!(adopted.files[0].root, "content");
        assert_eq!(
            fs::read(root.join("core.lua")).expect("adopted file"),
            b"manual"
        );
    });
}

#[test]
fn linked_content_roots_are_rejected() {
    let (temp, game, appdata, _module) = setup();
    let archive_path = temp.path().join("mod.zip");
    archive(&archive_path, &[("core.lua", b"data")]);
    fs::create_dir_all(appdata.join("Fixture")).expect("fixture");
    let target = appdata.join("Fixture/Mods");
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&game, &target).expect("junction fixture");
    #[cfg(not(windows))]
    std::os::unix::fs::symlink(&game, &target).expect("symlink fixture");
    let directory = module(
        temp.path(),
        &manifest(
            2,
            r#","content_root":{"base":"appdata","path":"Fixture/Mods"}"#,
        ),
        SCRIPT,
    );
    with_module_source(Some(&directory), || {
        assert!(preview_install_zip_with_appdata(GAME, &game, &archive_path, &appdata).is_err());
    });
}
