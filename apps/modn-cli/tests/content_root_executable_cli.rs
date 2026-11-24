//! A content-root game honors `--allow-executable` in its preview and install.
//!
//! The fixture module maps every archive member to the same destination under
//! its declared appdata content root. `tools/scc.exe` is therefore both the
//! archive member and the approved operation destination.

#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/content_root.rs"]
mod support;

use std::fs;
use support::{Fixture, GAME_ID};

const TOOL: &str = "tools/scc.exe";
const OTHER_TOOL: &str = "tools/other.exe";
const CATALOG: &str = "catalog-tool";
const STACK_TOP: &str = "stack-top";
const STACK_BASE: &str = "stack-base";

/// API 3 policy that maps every archive file to a content-root destination.
const IDENTITY_SCRIPT: &str = r#"
local fs = mods_host.fs

return {
    game_id = "content-game", api_version=3, version = "1.0.0",
    marker = "ContentGame.exe", common_paths = {},
    validate = function()
        local ok, info = pcall(fs.metadata, 1, "ContentGame.exe")
        return ok and info.kind == "file"
    end,
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

/// API 3 policy that installs `core.lua` in the game root and every other
/// archive member under the declared content root.
const MIXED_SCRIPT: &str = r#"
local fs = mods_host.fs

return {
    game_id = "content-game", api_version=3, version = "1.0.0",
    marker = "ContentGame.exe", common_paths = {},
    validate = function()
        local ok, info = pcall(fs.metadata, 1, "ContentGame.exe")
        return ok and info.kind == "file"
    end,
    steam_name = function() return "Content Game" end,
    is_epic = function() return false end,
    scan = function() return { mods = {}, framework_signals = {}, incomplete = false, redmod_available = false } end,
    doctor = function() return {} end,
    plan = function(files)
        local operations = {}
        for _, file in ipairs(files) do
            local root, kind = "content", "content-mod"
            if file.source == "core.lua" then
                root, kind = "game", "game-mod"
            end
            table.insert(operations, {
                source = file.source, destination = file.source,
                kind = kind, package_root = "", root = root,
            })
        end
        return { schema_version = 1, operations = operations, blockers = {} }
    end,
}
"#;

fn fixture() -> Fixture {
    let fixture = Fixture::new();
    fs::write(
        fixture.modules.join(GAME_ID).join("module.lua"),
        IDENTITY_SCRIPT,
    )
    .expect("content-root module script");
    fixture.register();
    fixture
}

fn preview(fixture: &Fixture, archive: &std::path::Path, extra: &[&str]) -> serde_json::Value {
    let mut args = vec![
        "--json",
        "install",
        archive.to_str().expect("archive"),
        "--dry-run",
        "--path",
        fixture.game.to_str().expect("game"),
    ];
    args.extend_from_slice(extra);
    fixture.json(&args)
}

fn codes(plan: &serde_json::Value) -> Vec<&str> {
    plan["plan"]["blockers"]
        .as_array()
        .expect("blockers")
        .iter()
        .map(|issue| issue["code"].as_str().expect("code"))
        .collect()
}

fn catalog_install_args<'a>(game: &'a str, archive: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "--json", "install", CATALOG, "--file", archive, "--path", game,
    ];
    args.extend_from_slice(extra);
    args
}

fn catalog_preview_args<'a>(game: &'a str, archive: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = catalog_install_args(game, archive, extra);
    args.push("--dry-run");
    args
}

fn stack_install_args<'a>(
    game: &'a str,
    top: &'a str,
    base: &'a str,
    extra: &[&'a str],
) -> Vec<&'a str> {
    let mut args = vec![
        "--json", "install", STACK_TOP, "--file", top, "--file", base, "--path", game,
    ];
    args.extend_from_slice(extra);
    args
}

fn stack_preview_args<'a>(
    game: &'a str,
    top: &'a str,
    base: &'a str,
    extra: &[&'a str],
) -> Vec<&'a str> {
    let mut args = stack_install_args(game, top, base, extra);
    args.push("--dry-run");
    args
}

#[test]
fn catalog_install_preview_and_install_honor_the_matching_approval() {
    let mut fixture = fixture();
    let (archive, hash) = fixture.archive("catalog-tool", &[(TOOL, b"tool")]);
    fixture.bucket();
    fixture.package(CATALOG, &hash, &[]);
    let game = fixture.game.to_str().expect("game");
    let archive = archive.to_str().expect("archive");

    for extra in [
        &[][..],
        &["--allow-executable", OTHER_TOOL][..],
        &["--allow-executable", "tools/missing.exe"][..],
    ] {
        let plan = fixture.json(&catalog_preview_args(game, archive, extra));
        assert_eq!(plan["plan"]["status"], "blocked", "{plan}");
        assert!(codes(&plan).contains(&"EXECUTABLE_PAYLOAD"), "{plan}");
    }

    let preview = fixture.json(&catalog_preview_args(
        game,
        archive,
        &["--allow-executable", TOOL],
    ));
    assert_eq!(preview["plan"]["status"], "reviewable", "{preview}");
    assert!(preview["plan"]["blockers"]
        .as_array()
        .is_some_and(Vec::is_empty));
    assert_eq!(preview["plan"]["operations"][0]["destination"], TOOL);
    assert_eq!(preview["plan"]["operations"][0]["root"], "content");

    let installed = fixture.json(&catalog_install_args(
        game,
        archive,
        &["--yes", "--allow-executable", TOOL],
    ));
    assert_eq!(installed["installed"]["files"][0]["root"], "content");
    assert_eq!(installed["installed"]["files"][0]["destination"], TOOL);
    assert_eq!(
        fs::read(fixture.content(TOOL)).expect("installed executable"),
        b"tool"
    );
    assert!(
        !fixture.game.join(TOOL).exists(),
        "the executable must stay outside the game root"
    );
}

#[test]
fn stack_install_preview_and_install_honor_the_matching_approval() {
    let mut fixture = fixture();
    fs::write(
        fixture.modules.join(GAME_ID).join("module.lua"),
        MIXED_SCRIPT,
    )
    .expect("mixed content-root module script");
    let (base, base_hash) = fixture.archive("stack-base", &[("core.lua", b"core")]);
    let (top, top_hash) = fixture.archive("stack-top", &[(TOOL, b"tool")]);
    fixture.bucket();
    fixture.package(STACK_BASE, &base_hash, &[]);
    fixture.package(STACK_TOP, &top_hash, &[STACK_BASE]);
    let game = fixture.game.to_str().expect("game");
    let top = top.to_str().expect("top");
    let base = base.to_str().expect("base");

    for extra in [
        &[][..],
        &["--allow-executable", OTHER_TOOL][..],
        &["--allow-executable", "tools/missing.exe"][..],
    ] {
        let plan = fixture.json(&stack_preview_args(game, top, base, extra));
        assert_eq!(plan["plan"]["status"], "blocked", "{plan}");
        assert!(codes(&plan).contains(&"EXECUTABLE_PAYLOAD"), "{plan}");
    }

    let preview = fixture.json(&stack_preview_args(
        game,
        top,
        base,
        &["--allow-executable", TOOL],
    ));
    assert_eq!(preview["plan"]["status"], "reviewable", "{preview}");
    assert!(preview["plan"]["blockers"]
        .as_array()
        .is_some_and(Vec::is_empty));
    assert_eq!(preview["plan"]["operations"][0]["destination"], TOOL);
    assert_eq!(preview["plan"]["operations"][0]["root"], "content");

    let installed = fixture.json(&stack_install_args(
        game,
        top,
        base,
        &["--yes", "--allow-executable", TOOL],
    ));
    let member = installed["stack"]
        .as_array()
        .expect("stack")
        .iter()
        .find(|member| member["installed"]["mod_id"] == STACK_TOP)
        .expect("installed top member");
    assert_eq!(member["installed"]["files"][0]["root"], "content");
    assert_eq!(member["installed"]["files"][0]["destination"], TOOL);
    assert_eq!(
        fs::read(fixture.content(TOOL)).expect("installed executable"),
        b"tool"
    );
    assert!(
        !fixture.game.join(TOOL).exists(),
        "the executable must stay outside the game root"
    );
}

#[test]
fn content_root_preview_blocks_an_executable_without_the_matching_approval() {
    let fixture = fixture();
    let (archive, _) = fixture.archive("tool", &[(TOOL, b"tool")]);

    for extra in [&[][..], &["--allow-executable", OTHER_TOOL][..]] {
        let plan = preview(&fixture, &archive, extra);
        assert_eq!(plan["status"], "blocked", "{plan}");
        assert!(
            plan["blockers"]
                .as_array()
                .expect("blockers")
                .iter()
                .any(|issue| issue["code"] == "EXECUTABLE_PAYLOAD"),
            "{plan}"
        );
    }
}

#[test]
fn content_root_preview_and_install_accept_the_matching_approval() {
    let fixture = fixture();
    let (archive, _) = fixture.archive("tool", &[(TOOL, b"tool")]);

    let plan = preview(&fixture, &archive, &["--allow-executable", TOOL]);
    assert_eq!(plan["status"], "reviewable", "{plan}");
    assert!(plan["blockers"].as_array().is_some_and(Vec::is_empty));
    assert_eq!(plan["operations"][0]["destination"], TOOL);
    assert_eq!(plan["operations"][0]["root"], "content");

    let installed = fixture.json(&[
        "--json",
        "install",
        archive.to_str().expect("archive"),
        "--yes",
        "--allow-executable",
        TOOL,
        "--path",
        fixture.game.to_str().expect("game"),
    ]);
    assert_eq!(installed["files"][0]["root"], "content");
    assert_eq!(
        fs::read(fixture.content(TOOL)).expect("installed executable"),
        b"tool"
    );
    assert!(
        !fixture.game.join(TOOL).exists(),
        "the executable must stay outside the game root"
    );
}
