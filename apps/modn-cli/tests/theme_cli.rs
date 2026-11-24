//! End-to-end tests for the `modn theme` command group.
//!
//! Every command runs the real `modn` binary against a temporary appdata base
//! and a temporary state directory. `LOCALAPPDATA`, `APPDATA` and
//! `MODN_APPDATA` all point at the temporary base, so no test reads or writes
//! the user's real Modn state.

#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "theme_support/fixture.rs"]
mod fixture;
#[path = "theme_support/check_register_tests.rs"]
mod check_register;
#[path = "theme_support/list_tests.rs"]
mod list;
#[path = "theme_support/resolve_tests.rs"]
mod resolve;
#[path = "theme_support/selection_tests.rs"]
mod selection;

use fixture::{Fixture, GAME};
use std::process::Command;

fn run_without_prefix(args: &[&str]) -> std::process::Output {
    Command::new(main_bucket::modn_binary())
        .args(args)
        .output()
        .expect("run modn")
}

#[test]
fn the_top_level_help_lists_theme_and_its_subcommands() {
    let top = run_without_prefix(&["--help"]);
    assert!(top.status.success());
    let top = String::from_utf8_lossy(&top.stdout);
    assert!(top.contains("theme"), "{top}");

    let theme = run_without_prefix(&["theme", "--help"]);
    assert!(theme.status.success());
    let theme = String::from_utf8_lossy(&theme.stdout);
    for command in [
        "check", "register", "list", "remove", "select", "reset", "resolve",
    ] {
        assert!(theme.contains(command), "missing {command}:\n{theme}");
    }
}

#[test]
fn desktop_theme_list_parses() {
    let fixture = Fixture::new();
    let listed = fixture.json(&["theme", "list"]);
    assert!(listed["themes"].is_array());
}

#[test]
fn desktop_theme_list_with_game_parses() {
    let fixture = Fixture::new();
    let listed = fixture.json(&["theme", "list", "--game", GAME]);
    assert!(listed["themes"].is_array());
}

#[test]
fn desktop_theme_resolve_with_game_parses() {
    let fixture = Fixture::new();
    let resolved = fixture.json(&["theme", "resolve", "--game", GAME]);
    assert_eq!(resolved["game"], GAME);
}

#[test]
fn desktop_theme_select_with_and_without_variant_parses() {
    let fixture = Fixture::new();
    let theme = fixture.source_theme(GAME, "night-city", &["neon"]);
    fixture.register(&theme);
    let selected = fixture.json(&["theme", "select", GAME, "night-city"]);
    assert_eq!(selected["selected"]["variant"], serde_json::Value::Null);
    let variant = fixture.json(&[
        "theme", "select", GAME, "night-city", "--variant", "neon",
    ]);
    assert_eq!(variant["selected"]["variant"], "neon");
}

#[test]
fn desktop_theme_reset_with_and_without_game_parses() {
    let fixture = Fixture::new();
    let theme = fixture.source_theme(GAME, "night-city", &[]);
    fixture.register(&theme);
    fixture.json(&["theme", "select", GAME, "night-city"]);
    let one = fixture.json(&["theme", "reset", "--game", GAME]);
    assert_eq!(one["reset"], serde_json::json!([GAME]));
    let all = fixture.json(&["theme", "reset"]);
    assert!(all["reset"].is_array());
}

#[test]
fn desktop_theme_register_parses() {
    let fixture = Fixture::new();
    let theme = fixture.source_theme(GAME, "night-city", &[]);
    let registered = fixture.json(&["theme", "register", theme.to_str().expect("theme path")]);
    assert_eq!(registered["registered"]["id"], "night-city");
}

#[test]
fn theme_commands_never_create_state_sqlite() {
    let fixture = Fixture::new();
    fixture.json(&["theme", "list"]);
    fixture.json(&["theme", "resolve", "--game", GAME]);
    let example = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/themes/cyberpunk-2077/night-city");
    fixture.json(&["theme", "check", example.to_str().expect("example path")]);
    assert!(!fixture.state.join("state.sqlite").exists());
}
