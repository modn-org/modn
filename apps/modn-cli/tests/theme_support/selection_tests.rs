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

#[test]
fn select_none_stores_the_explicit_default_choice() {
    let fixture = Fixture::new();
    fixture.official_theme(GAME, "a-official", &[]);
    let selected = fixture.json(&["theme", "select", GAME, "--none"]);
    assert_eq!(
        selected["selected"],
        serde_json::json!({"game": GAME, "theme": null, "variant": null})
    );
    let keys: Vec<_> = selected["selected"]
        .as_object()
        .expect("selected object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["game", "theme", "variant"]);
}

#[test]
fn select_none_prints_the_documented_sentence() {
    let fixture = Fixture::new();
    let output = fixture.run_human(&["theme", "select", GAME, "--none"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("Selected no theme for {GAME}.")
    );
}

#[test]
fn select_none_needs_no_theme_or_registry() {
    let fixture = Fixture::new();
    let selected = fixture.json(&["theme", "select", GAME, "--none"]);
    assert_eq!(selected["selected"]["theme"], serde_json::Value::Null);
    assert!(!fixture.state.join("themes").join("registry").exists());
}

#[test]
fn select_none_conflicts_with_a_theme_and_a_variant() {
    let fixture = Fixture::new();
    let theme = fixture.source_theme(GAME, "night-city", &["neon"]);
    fixture.register(&theme);

    let with_theme = fixture.run(&["theme", "select", GAME, "night-city", "--none"]);
    assert_eq!(with_theme.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&with_theme.stderr);
    assert!(stderr.contains("--none"), "{stderr}");

    let with_variant = fixture.run(&["theme", "select", GAME, "--none", "--variant", "neon"]);
    assert_eq!(with_variant.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&with_variant.stderr);
    assert!(stderr.contains("--none"), "{stderr}");
}

#[test]
fn select_without_a_theme_or_none_is_a_usage_error() {
    let fixture = Fixture::new();
    let output = fixture.run(&["theme", "select", GAME]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--none"), "{stderr}");
}

#[test]
fn select_none_refuses_an_invalid_game_id_like_select() {
    let fixture = Fixture::new();
    let none_error = fixture.error_json(&["theme", "select", "not a game", "--none"]);
    let select_error = fixture.error_json(&["theme", "select", "not a game", "night-city"]);
    assert_eq!(none_error["error"], select_error["error"], "{none_error}");
    assert!(
        none_error["message"]
            .as_str()
            .is_some_and(|message| message.contains("not a valid")),
        "{none_error}"
    );
}

#[test]
fn reset_clears_a_none_selection_and_the_official_theme_applies() {
    let fixture = Fixture::new();
    fixture.official_theme(GAME, "a-official", &[]);
    fixture.json(&["theme", "select", GAME, "--none"]);
    let reset = fixture.json(&["theme", "reset", "--game", GAME]);
    assert_eq!(reset["reset"], serde_json::json!([GAME]));
    let resolved = fixture.json(&["theme", "resolve", "--game", GAME]);
    assert_eq!(resolved["theme"]["id"], "a-official");
    assert_eq!(resolved["theme"]["origin"], "official");
}
