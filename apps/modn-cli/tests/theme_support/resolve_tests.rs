use crate::fixture::{Fixture, GAME};
use serde_json::Value;
use std::collections::BTreeSet;

fn keys(value: &Value) -> BTreeSet<&str> {
    value
        .as_object()
        .expect("JSON object")
        .keys()
        .map(String::as_str)
        .collect()
}

fn assert_keys(value: &Value, expected: &[&str]) {
    let expected: BTreeSet<&str> = expected.iter().copied().collect();
    assert_eq!(keys(value), expected, "{value}");
}

fn selected_fixture() -> Fixture {
    let fixture = Fixture::new();
    fixture.official_theme(GAME, "a-official", &[]);
    fixture.official_theme(GAME, "z-official", &[]);
    let local = fixture.source_theme(GAME, "night-city", &["neon"]);
    fixture.register(&local);
    fixture.json(&["theme", "select", GAME, "night-city", "--variant", "neon"]);
    fixture
}

#[test]
fn resolve_returns_the_selected_theme_and_variant() {
    let fixture = selected_fixture();
    let resolved = fixture.json(&["theme", "resolve", "--game", GAME]);
    assert_eq!(resolved["game"], GAME);
    assert_eq!(resolved["theme"]["id"], "night-city");
    assert_eq!(resolved["theme"]["origin"], "selected");
    assert_eq!(resolved["theme"]["variant"], "neon");
    assert_eq!(resolved["theme"]["source"]["kind"], "local");
    assert!(resolved["theme"]["css"]
        .as_str()
        .is_some_and(|css| !css.is_empty()));
    assert!(resolved["theme"]["variant_css"].as_str().is_some());
    assert!(std::path::Path::new(
        resolved["theme"]["assets_dir"]
            .as_str()
            .expect("assets dir")
    )
    .is_absolute());
    assert_eq!(resolved["fallback"], Value::Null);
    assert_eq!(resolved["diagnostics"], serde_json::json!([]));
}

#[test]
fn resolve_falls_back_to_the_first_official_theme_after_removal() {
    let fixture = selected_fixture();
    fixture.json(&["theme", "remove", "night-city", "--game", GAME]);
    let resolved = fixture.json(&["theme", "resolve", "--game", GAME]);
    assert_eq!(resolved["theme"]["id"], "a-official");
    assert_eq!(resolved["theme"]["origin"], "official");
    assert_eq!(resolved["theme"]["variant"], Value::Null);
    assert_eq!(resolved["theme"]["variant_css"], Value::Null);
    assert_eq!(resolved["fallback"]["reason"], "theme-missing");
    assert!(resolved["fallback"]["message"].as_str().is_some());
    assert!(
        resolved["diagnostics"]
            .as_array()
            .is_some_and(|items| { items.iter().any(|item| item["rule"] == "theme-missing") }),
        "{resolved}"
    );
}

#[test]
fn resolve_without_a_selection_or_official_theme_returns_null() {
    let fixture = Fixture::new();
    let resolved = fixture.json(&["theme", "resolve", "--game", GAME]);
    assert_eq!(resolved["game"], GAME);
    assert_eq!(resolved["theme"], Value::Null);
    assert_eq!(resolved["fallback"], Value::Null);
    assert_eq!(resolved["diagnostics"], serde_json::json!([]));
}

#[test]
fn resolve_requires_a_game() {
    let fixture = Fixture::new();
    let output = fixture.failed(&["theme", "resolve"]);
    assert_eq!(output.status.code(), Some(1));
    let value: Value = serde_json::from_slice(&output.stderr).expect("error JSON");
    assert!(
        value["message"]
            .as_str()
            .is_some_and(|message| message.contains("--game")),
        "{value}"
    );
}

#[test]
fn list_and_resolve_key_sets_match_the_documented_examples() {
    let fixture = selected_fixture();
    fixture.broken_official_theme(GAME, "broken-city");
    let listed = fixture.json(&["theme", "list", "--game", GAME]);
    assert_keys(&listed, &["themes", "diagnostics"]);
    assert_keys(
        &listed["themes"][0],
        &[
            "id",
            "game",
            "name",
            "theme_api",
            "variants",
            "icon",
            "source",
            "selected",
        ],
    );
    assert_keys(&listed["themes"][0]["source"], &["kind", "bucket", "label"]);
    assert_keys(
        &listed["diagnostics"][0],
        &["theme", "path", "line", "rule", "message"],
    );

    let resolved = fixture.json(&["theme", "resolve", "--game", GAME]);
    assert_keys(&resolved, &["game", "theme", "fallback", "diagnostics"]);
    assert_keys(
        &resolved["theme"],
        &[
            "id",
            "name",
            "source",
            "variant",
            "origin",
            "css",
            "variant_css",
            "assets_dir",
        ],
    );
    assert_keys(&resolved["theme"]["source"], &["kind", "bucket", "label"]);

    fixture.json(&["theme", "remove", "night-city", "--game", GAME]);
    let fallback = fixture.json(&["theme", "resolve", "--game", GAME]);
    assert_keys(&fallback["fallback"], &["reason", "message"]);
}

#[test]
fn resolve_returns_no_theme_after_select_none_with_an_official_theme() {
    let fixture = Fixture::new();
    fixture.official_theme(GAME, "a-official", &[]);
    fixture.json(&["theme", "select", GAME, "--none"]);
    let resolved = fixture.json(&["theme", "resolve", "--game", GAME]);
    assert_eq!(resolved["game"], GAME);
    assert_eq!(resolved["theme"], Value::Null);
    assert_eq!(resolved["fallback"], Value::Null);
    assert_eq!(resolved["diagnostics"], serde_json::json!([]));
}

#[test]
fn list_marks_no_theme_selected_after_select_none() {
    let fixture = Fixture::new();
    fixture.official_theme(GAME, "a-official", &[]);
    fixture.json(&["theme", "select", GAME, "--none"]);
    let listed = fixture.json(&["theme", "list", "--game", GAME]);
    let themes = listed["themes"].as_array().expect("themes");
    assert!(!themes.is_empty(), "{listed}");
    assert!(
        themes.iter().all(|theme| theme["selected"] == false),
        "{listed}"
    );
    let human = fixture.run_human(&["theme", "list", "--game", GAME]);
    assert!(human.status.success());
    let stdout = String::from_utf8_lossy(&human.stdout);
    assert!(!stdout.contains("(selected)"), "{stdout}");
}
