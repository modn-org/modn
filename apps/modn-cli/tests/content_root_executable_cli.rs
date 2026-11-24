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
