//! `theme select`, `theme remove` and `theme reset` behavior.

use crate::fixture::{Fixture, GAME};

#[test]
fn select_accepts_a_catalog_theme_and_variant() {
    let fixture = Fixture::new();
    let theme = fixture.source_theme(GAME, "night-city", &["neon"]);
    fixture.register(&theme);
    let selected = fixture.json(&["theme", "select", GAME, "night-city", "--variant", "neon"]);
    assert_eq!(selected["selected"]["game"], GAME);
    assert_eq!(selected["selected"]["theme"], "night-city");
    assert_eq!(selected["selected"]["variant"], "neon");
}

#[test]
fn select_refuses_an_unknown_theme_or_variant() {
    let fixture = Fixture::new();
    let theme = fixture.source_theme(GAME, "night-city", &["neon"]);
    fixture.register(&theme);

    let unknown_theme = fixture.error_json(&["theme", "select", GAME, "ghost-city"]);
    assert!(unknown_theme["error"]
        .as_str()
        .is_some_and(|code| !code.is_empty()));
    assert!(
        unknown_theme["message"]
            .as_str()
            .is_some_and(|message| message.contains("ghost-city")),
        "{unknown_theme}"
    );

    let unknown_variant =
        fixture.error_json(&["theme", "select", GAME, "night-city", "--variant", "noir"]);
    assert!(unknown_variant["error"]
        .as_str()
        .is_some_and(|code| !code.is_empty()));
    assert!(
        unknown_variant["message"]
            .as_str()
            .is_some_and(|message| message.contains("noir")),
        "{unknown_variant}"
    );
}

#[test]
fn remove_deletes_one_registration() {
    let fixture = Fixture::new();
    let theme = fixture.source_theme(GAME, "night-city", &[]);
    fixture.register(&theme);
    let removed = fixture.json(&["theme", "remove", "night-city", "--game", GAME]);
    assert_eq!(removed["removed"]["id"], "night-city");
    assert_eq!(removed["removed"]["game"], GAME);
    assert!(!fixture.registry_entry(GAME, "night-city").exists());
}

#[test]
fn remove_refuses_an_unregistered_id() {
    let fixture = Fixture::new();
    let output = fixture.failed(&["theme", "remove", "ghost-city"]);
    assert_eq!(output.status.code(), Some(1));
    let value: serde_json::Value = serde_json::from_slice(&output.stderr).expect("error JSON");
    assert!(
        value["message"]
            .as_str()
            .is_some_and(|message| message.contains("ghost-city")
                && message.contains("registered")),
        "{value}"
    );
}

#[test]
fn remove_without_a_game_refuses_an_ambiguous_id() {
    let fixture = Fixture::new();
    for game in ["alpha", "beta"] {
        let theme = fixture.source_theme(game, "night-city", &[]);
        fixture.register(&theme);
    }
    let output = fixture.failed(&["theme", "remove", "night-city"]);
    assert_eq!(output.status.code(), Some(1));
    let value: serde_json::Value = serde_json::from_slice(&output.stderr).expect("error JSON");
    let message = value["message"].as_str().expect("message");
    assert!(message.contains("several games"), "{message}");
    assert!(
        message.contains("alpha") && message.contains("beta"),
        "{message}"
    );
    assert!(fixture.registry_entry("alpha", "night-city").is_dir());
    assert!(fixture.registry_entry("beta", "night-city").is_dir());

    let removed = fixture.json(&["theme", "remove", "night-city", "--game", "alpha"]);
    assert_eq!(removed["removed"]["game"], "alpha");
    assert!(!fixture.registry_entry("alpha", "night-city").exists());
    assert!(fixture.registry_entry("beta", "night-city").is_dir());
}

#[test]
fn reset_clears_one_game_then_every_game() {
    let fixture = Fixture::new();
    for game in ["alpha", "beta"] {
        let theme = fixture.source_theme(game, "night-city", &[]);
        fixture.register(&theme);
        fixture.json(&["theme", "select", game, "night-city"]);
    }
    let one = fixture.json(&["theme", "reset", "--game", "alpha"]);
    assert_eq!(one["reset"], serde_json::json!(["alpha"]));
    let all = fixture.json(&["theme", "reset"]);
    assert_eq!(all["reset"], serde_json::json!(["beta"]));
}
