//! Policy-level scan contract for inaccessible but already discovered plugin roots.
use mlua::{Function, Lua, Table, Value};
use std::fs;
use std::path::Path;

#[test]
fn discovered_unreadable_red4ext_plugin_marks_scan_incomplete() {
    let lua = Lua::new();
    let host = lua.create_table().expect("host table");
    host.set("api_version", 1u32).expect("host version");
    let fs_host = lua.create_table().expect("fs host");
    let metadata = lua
        .create_function(|_, (_root, _path): (u32, String)| Ok(Value::Nil))
        .expect("metadata stub");
    fs_host.set("metadata", metadata).expect("set metadata");
    host.set("fs", fs_host).expect("set fs");
    let directory = lua.create_table().expect("directory host");
    let list = lua
        .create_function(|lua, (_root, path): (u32, String)| {
            if path == "red4ext/plugins/Unreadable" {
                return Err(mlua::Error::RuntimeError("simulated access denial".into()));
            }
            let response = lua.create_table()?;
            let items = lua.create_table()?;
            if path == "red4ext/plugins" {
                let item = lua.create_table()?;
                item.set("name", "Unreadable")?;
                item.set("kind", "directory")?;
                items.set(1, item)?;
            }
            response.set("items", items)?;
            response.set("incomplete", false)?;
            Ok(response)
        })
        .expect("list stub");
    directory.set("list", list).expect("set list");
    host.set("directory", directory).expect("set directory");
    lua.globals().set("mods_host", host).expect("set host");

    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("game-modules/cyberpunk-2077/main.luau"),
    )
    .expect("bundled policy");
    let policy: Table = lua.load(&source).eval().expect("evaluate bundled Luau");
    let report: Table = policy
        .get::<Function>("scan")
        .expect("scan method")
        .call(100u32)
        .expect("scan result");
    assert!(
        report.get::<bool>("incomplete").expect("incomplete"),
        "observed but unreadable plugin directories cannot be silently skipped"
    );
}
