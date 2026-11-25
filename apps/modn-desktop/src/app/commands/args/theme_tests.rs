use super::tests::{ok, strings};
use super::*;
use std::path::Path;

#[test]
fn theme_list_selects_one_game_or_every_game() {
    assert_eq!(strings(ok(theme_list(None))), vec!["theme", "list"]);
    assert_eq!(
        strings(ok(theme_list(Some("cyberpunk-2077")))),
        vec!["theme", "list", "--game", "cyberpunk-2077"]
    );
}

#[test]
fn theme_resolve_names_the_game() {
    assert_eq!(
        strings(ok(theme_resolve("cyberpunk-2077"))),
        vec!["theme", "resolve", "--game", "cyberpunk-2077"]
    );
}

#[test]
fn theme_select_names_the_pair_and_an_optional_variant() {
    assert_eq!(
        strings(ok(theme_select("cyberpunk-2077", "night-city", None))),
        vec!["theme", "select", "cyberpunk-2077", "night-city"]
    );
    assert_eq!(
        strings(ok(theme_select(
            "cyberpunk-2077",
            "night-city",
            Some("neon")
        ))),
        vec![
            "theme",
            "select",
            "cyberpunk-2077",
            "night-city",
            "--variant",
            "neon"
        ]
    );
}

#[test]
fn theme_select_none_names_the_explicit_default() {
    assert_eq!(
        strings(ok(theme_select_none("cyberpunk-2077"))),
        vec!["theme", "select", "cyberpunk-2077", "--none"]
    );
}

#[test]
fn theme_select_none_refuses_an_invalid_game_id() {
    let error = theme_select_none("Night-City").expect_err("an invalid id must be refused");
    assert_eq!(error.code, "INVALID_ID");
}

#[test]
fn theme_reset_selects_one_game_or_every_game() {
    assert_eq!(strings(ok(theme_reset(None))), vec!["theme", "reset"]);
    assert_eq!(
        strings(ok(theme_reset(Some("cyberpunk-2077")))),
        vec!["theme", "reset", "--game", "cyberpunk-2077"]
    );
}

#[test]
fn theme_register_names_the_chosen_folder() {
    assert_eq!(
        strings(theme_register(Path::new(r"C:\themes\night-city"))),
        vec!["theme", "register", r"C:\themes\night-city"]
    );
}

#[test]
fn theme_ids_must_match_the_documented_grammar() {
    let max = "a".repeat(64);
    let too_long = "a".repeat(65);
    for valid in [
        "a",
        "9",
        "night-city",
        "cyberpunk-2077",
        "a1-b2",
        max.as_str(),
    ] {
        assert!(valid_id(valid), "{valid} must be accepted");
    }
    for invalid in [
        "",
        "-night",
        "night-",
        "night--city",
        "Night-City",
        "night_city",
        "night city",
        "night/city",
        "night.city",
        too_long.as_str(),
    ] {
        assert!(!valid_id(invalid), "{invalid} must be refused");
    }
}

#[test]
fn every_theme_builder_refuses_an_invalid_id() {
    let refused = [
        theme_list(Some("Night-City")).map(strings),
        theme_resolve("Night-City").map(strings),
        theme_select("Night-City", "night-city", None).map(strings),
        theme_select("cyberpunk-2077", "Night-City", None).map(strings),
        theme_select("cyberpunk-2077", "night-city", Some("Neon")).map(strings),
        theme_reset(Some("Night-City")).map(strings),
    ];
    for result in refused {
        let error = result.expect_err("an invalid id must be refused");
        assert_eq!(error.code, "INVALID_ID");
    }
}
