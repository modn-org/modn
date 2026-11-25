use crate::fixture::{Fixture, GAME};

#[test]
fn list_shows_official_local_and_bucket_themes_and_broken_diagnostics() {
    let fixture = Fixture::new();
    fixture.official_theme(GAME, "official-city", &[]);
    fixture.broken_official_theme(GAME, "broken-city");
    fixture.bucket_theme("community", GAME, "bucket-city", &[]);
    let local = fixture.source_theme(GAME, "night-city", &["neon"]);
    fixture.register(&local);
    fixture.json(&["theme", "select", GAME, "night-city", "--variant", "neon"]);

    let listed = fixture.json(&["theme", "list", "--game", GAME]);
    let themes = listed["themes"].as_array().expect("themes");
    let ids: Vec<&str> = themes
        .iter()
        .map(|theme| theme["id"].as_str().expect("theme id"))
        .collect();
    assert_eq!(
        ids,
        ["official-city", "night-city", "bucket-city"],
        "{listed}"
    );

    let official = &themes[0];
    assert_eq!(official["source"]["kind"], "official");
    assert_eq!(official["source"]["bucket"], "main");
    assert_eq!(official["source"]["label"], "main bucket");
    assert_eq!(official["selected"], false);

    let local = &themes[1];
    assert_eq!(local["source"]["kind"], "local");
    assert_eq!(local["source"]["bucket"], serde_json::Value::Null);
    assert_eq!(local["selected"], true);

    let bucket = &themes[2];
    assert_eq!(bucket["source"]["kind"], "bucket");
    assert_eq!(bucket["source"]["bucket"], "community");
    assert_eq!(bucket["source"]["label"], "community");
    assert_eq!(bucket["selected"], false);

    assert!(!ids.contains(&"broken-city"));
    let diagnostics = listed["diagnostics"].as_array().expect("diagnostics");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic["theme"] == "cyberpunk-2077/broken-city"
                && diagnostic["rule"] == "forbidden-position-fixed"
        }),
        "{listed}"
    );
}

#[test]
fn list_with_a_game_filter_names_no_other_game() {
    let fixture = Fixture::new();
    fixture.official_theme("other-game", "other-city", &[]);
    fixture.official_theme(GAME, "night-city", &[]);
    let listed = fixture.json(&["theme", "list", "--game", GAME]);
    let themes = listed["themes"].as_array().expect("themes");
    assert_eq!(themes.len(), 1, "{listed}");
    assert_eq!(themes[0]["id"], "night-city");
}
