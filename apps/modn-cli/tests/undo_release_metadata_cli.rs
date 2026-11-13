//! Undo restores the catalog identity, dependency links, declared conflicts
//! and native-injector status that the history record holds.

#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/catalog.rs"]
mod support;

#[path = "common/release_metadata.rs"]
mod release_metadata;

use crate::support::{error, Fixture};
use release_metadata::{
    archive, cache, declare_conflicts, error_code, history, https, id_of, install, json_of, run,
    tool_module, write_manifest,
};
use serde_json::json;

#[test]
fn undo_of_an_update_restores_the_recorded_dependency_link() {
    let fixture = Fixture::new();
    let (base, base_hash) = archive(
        &fixture,
        "base",
        &[("Demo/package/plugins/base.plugin", "base")],
    );
    let (top_v1, top_v1_hash) = archive(
        &fixture,
        "top-v1",
        &[("Demo/package/plugins/top.plugin", "one")],
    );
    let (top_v2, top_v2_hash) = archive(
        &fixture,
        "top-v2",
        &[("Demo/package/plugins/top.plugin", "two")],
    );
    let (rival, rival_hash) = archive(
        &fixture,
        "rival",
        &[("Demo/package/plugins/rival.plugin", "rival")],
    );
    let base_url = "https://example.invalid/base-1.0.zip";
    let top_url = "https://example.invalid/top-1.0.zip";
    write_manifest(
        &fixture,
        "deps",
        "test-game",
        "base",
        "1.0",
        &base_hash,
        &[],
        &[],
        false,
        https(base_url),
    );
    write_manifest(
        &fixture,
        "deps",
        "test-game",
        "top",
        "1.0",
        &top_v1_hash,
        &["base"],
        &[],
        false,
        https(top_url),
    );
    declare_conflicts(&fixture, "deps", "test-game", "top", &["rival"]);
    write_manifest(
        &fixture,
        "deps",
        "test-game",
        "rival",
        "1.0",
        &rival_hash,
        &[],
        &[],
        false,
        json!({"kind": "manual", "filename": "rival.zip"}),
    );
    fixture.register();

    let installed_base = install(&fixture, "base", &base, &[]);
    assert!(
        installed_base.status.success(),
        "{}",
        error(&installed_base)
    );
    let installed_top = install(&fixture, "top", &top_v1, &[]);
    assert!(installed_top.status.success(), "{}", error(&installed_top));

    write_manifest(
        &fixture,
        "deps",
        "test-game",
        "top",
        "2.0",
        &top_v2_hash,
        &["base"],
        &[],
        false,
        https("https://example.invalid/top-2.0.zip"),
    );
    declare_conflicts(&fixture, "deps", "test-game", "top", &["rival"]);
    let updated = run(&fixture, &["--json", "update", "top", "--yes"], &[&top_v2]);
    assert!(updated.status.success(), "{}", error(&updated));
    cache(&fixture, "test-game", "top", "1.0", top_url, &top_v1);
    let operations = history(&fixture);
    let target = id_of(&operations[0]);
    assert_eq!(operations[0]["kind"], "update");

    let undone = fixture.run(&["--json", "undo", &target, "--yes"]);
    assert!(undone.status.success(), "{}", error(&undone));

    let refused = fixture.run(&["--json", "uninstall", "base", "--yes"]);
    assert!(
        !refused.status.success(),
        "the framework must be needed again"
    );
    assert_eq!(error_code(&refused), "PACKAGE_NEEDED");

    let conflict = install(&fixture, "rival", &rival, &[]);
    assert!(
        !conflict.status.success(),
        "the restored conflict blocks it"
    );
    assert!(
        error(&conflict).contains("conflict"),
        "{}",
        error(&conflict)
    );
    assert!(!fixture.game.join("addons/rival.plugin").exists());
}

#[test]
fn undo_of_an_uninstall_restores_catalog_identity_and_dependency_links() {
    let fixture = Fixture::new();
    let (base, base_hash) = archive(
        &fixture,
        "base",
        &[("Demo/package/plugins/base.plugin", "base")],
    );
    let (top_v1, top_v1_hash) = archive(
        &fixture,
        "top-v1",
        &[("Demo/package/plugins/top.plugin", "one")],
    );
    let (top_v2, top_v2_hash) = archive(
        &fixture,
        "top-v2",
        &[("Demo/package/plugins/top.plugin", "two")],
    );
    let top_url = "https://example.invalid/top-1.0.zip";
    write_manifest(
        &fixture,
        "deps",
        "test-game",
        "base",
        "1.0",
        &base_hash,
        &[],
        &[],
        false,
        https("https://example.invalid/base-1.0.zip"),
    );
    write_manifest(
        &fixture,
        "deps",
        "test-game",
        "top",
        "1.0",
        &top_v1_hash,
        &["base"],
        &[],
        false,
        https(top_url),
    );
    fixture.register();
    assert!(install(&fixture, "base", &base, &[]).status.success());
    assert!(install(&fixture, "top", &top_v1, &[]).status.success());
    cache(&fixture, "test-game", "top", "1.0", top_url, &top_v1);

    let removed = fixture.run(&["--json", "uninstall", "top", "--yes"]);
    assert!(removed.status.success(), "{}", error(&removed));
    let target = id_of(&history(&fixture)[0]);
    let undone = fixture.run(&["--json", "undo", &target, "--yes"]);
    assert!(undone.status.success(), "{}", error(&undone));

    let refused = fixture.run(&["--json", "uninstall", "base", "--yes"]);
    assert!(
        !refused.status.success(),
        "the restored dependency blocks it"
    );
    assert_eq!(error_code(&refused), "PACKAGE_NEEDED");

    let packages = json_of(&fixture.run(&["--json", "list"]));
    let top = packages
        .as_array()
        .expect("list")
        .iter()
        .find(|package| package["mod_id"] == "top")
        .expect("restored top package");
    assert_eq!(top["catalog"]["bucket"], "deps");
    assert_eq!(top["catalog"]["game_id"], "test-game");
    assert_eq!(top["catalog"]["mod_id"], "top");

    write_manifest(
        &fixture,
        "deps",
        "test-game",
        "top",
        "2.0",
        &top_v2_hash,
        &["base"],
        &[],
        false,
        https("https://example.invalid/top-2.0.zip"),
    );
    let updated = run(&fixture, &["--json", "update", "top", "--yes"], &[&top_v2]);
    assert!(updated.status.success(), "{}", error(&updated));
}

#[test]
fn undo_of_an_uninstall_restores_an_approved_tool_executable() {
    let fixture = Fixture::new();
    tool_module(&fixture);
    let (tool, tool_hash) = archive(&fixture, "tool", &[("engine/tools/scc.exe", "compiler")]);
    let url = "https://example.invalid/tool-1.0.zip";
    write_manifest(
        &fixture,
        "deps",
        "test-game",
        "tool",
        "1.0",
        &tool_hash,
        &[],
        &["engine/tools/scc.exe"],
        false,
        https(url),
    );
    fixture.register();
    cache(&fixture, "test-game", "tool", "1.0", url, &tool);
    let installed = install(
        &fixture,
        "tool",
        &tool,
        &["--allow-executable", "engine/tools/scc.exe"],
    );
    assert!(installed.status.success(), "{}", error(&installed));
    let executable = fixture.game.join("engine/tools/scc.exe");
    assert!(executable.is_file());

    assert!(fixture
        .run(&["uninstall", "tool", "--yes"])
        .status
        .success());
    assert!(!executable.exists());
    let target = id_of(&history(&fixture)[0]);
    let undone = fixture.run(&[
        "--json",
        "undo",
        &target,
        "--yes",
        "--allow-executable",
        "engine/tools/scc.exe",
    ]);
    assert!(undone.status.success(), "{}", error(&undone));
    assert!(executable.is_file());

    // The install stored the approval, and uninstall keeps it, so a second
    // restore needs no flag.
    assert!(fixture
        .run(&["uninstall", "tool", "--yes"])
        .status
        .success());
    let target = id_of(&history(&fixture)[0]);
    let stored = fixture.run(&["--json", "undo", &target, "--yes"]);
    assert!(stored.status.success(), "{}", error(&stored));
    assert!(executable.is_file());
}
