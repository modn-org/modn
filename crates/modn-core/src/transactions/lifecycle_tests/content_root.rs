use super::super::install::Fault;
use crate::state::Store;
use crate::transactions::NativeInjectorPolicy;
use std::fs::{self, File};
use std::io::Write;
use tempfile::tempdir;
use zip::write::SimpleFileOptions;

#[test]
fn content_root_rollback_and_recovery_remove_only_created_state() {
    let temp = tempdir().unwrap();
    let game = temp.path().join("game");
    let appdata = temp.path().join("appdata");
    fs::create_dir_all(&game).unwrap();
    fs::write(game.join("ContentGame.exe"), b"marker").unwrap();
    fs::create_dir_all(&appdata).unwrap();
    let module = temp.path().join("module");
    fs::create_dir_all(&module).unwrap();
    fs::write(
        module.join("module.json"),
        r#"{"game_id":"content-game","version":"1.0.0","api_version":2,"entrypoint":"module.lua","content_root":{"base":"appdata","path":"Fixture/Mods"}}"#,
    )
    .unwrap();
    fs::write(
        module.join("module.lua"),
        r#"return {
            game_id="content-game", api_version=2, version="1.0.0",
            marker="ContentGame.exe", common_paths={},
            validate=function() return true end,
            steam_name=function() return "Content Game" end,
            is_epic=function() return false end,
            scan=function() return {mods={},framework_signals={},incomplete=false,redmod_available=false} end,
            doctor=function() return {} end,
            plan=function(files)
                local operations={}
                for _, file in ipairs(files) do
                    table.insert(operations,{source=file.source,destination=file.source,kind="content-mod",package_root="",root="content"})
                end
                return {schema_version=1,operations=operations,blockers={}}
            end,
        }"#,
    )
    .unwrap();
    let archive_path = temp.path().join("mod.zip");
    let mut zip = zip::ZipWriter::new(File::create(&archive_path).unwrap());
    for (name, bytes) in [("a.lua", b"a" as &[u8]), ("b.lua", b"b")] {
        zip.start_file(name, SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
    let mut store = Store::open(&temp.path().join("state/state.sqlite")).unwrap();
    crate::game_modules::with_module_source(Some(&module), || {
        let plan = crate::planning::preview_install_zip_with_appdata(
            "content-game",
            &game,
            &archive_path,
            &appdata,
        )
        .unwrap();
        let error = store
            .install_operations_inner(&game, &archive_path, &plan, Some(1))
            .expect_err("injected failure");
        assert!(
            error.to_string().contains("injected interruption"),
            "{error}"
        );
        assert!(!appdata.join("Fixture/Mods/a.lua").exists());
        assert!(!appdata.join("Fixture/Mods/b.lua").exists());
        assert!(!appdata.join("Fixture/Mods").exists());
        store
            .install_locked(
                &game,
                &archive_path,
                &plan,
                Some(Fault::CrashAfter(1)),
                &[],
                None,
                NativeInjectorPolicy::Refuse,
            )
            .expect_err("simulated crash");
        assert!(appdata.join("Fixture/Mods/a.lua").exists());
        store.recover().expect("recover");
        assert!(!appdata.join("Fixture/Mods/a.lua").exists());
        assert!(!appdata.join("Fixture/Mods").exists());
    });
}
