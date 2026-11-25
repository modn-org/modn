use crate::fixture::{Fixture, GAME};
use serde_json::Value;
use std::fs;
use std::path::Path;

#[test]
fn check_accepts_a_valid_theme() {
    let fixture = Fixture::new();
    let theme = fixture.source_theme(GAME, "night-city", &["neon", "noir"]);
    let output = fixture.run(&["theme", "check", theme.to_str().expect("theme path")]);
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).expect("JSON output");
    assert_eq!(value["ok"], true, "{value}");
    assert_eq!(value["theme"]["id"], "night-city");
    assert_eq!(value["theme"]["game"], GAME);
    assert_eq!(value["theme"]["theme_api"], 1);
    assert_eq!(value["theme"]["source"]["kind"], "local");
    assert_eq!(value["theme"]["source"]["bucket"], Value::Null);
    assert_eq!(value["theme"]["source"]["label"], "local registry");
    assert_eq!(value["theme"]["selected"], false);
    assert_eq!(value["diagnostics"], serde_json::json!([]));
}

#[test]
fn check_without_json_prints_a_short_human_report() {
    let fixture = Fixture::new();
    let theme = fixture.source_theme(GAME, "night-city", &[]);
    let output = fixture.run_human(&["theme", "check", theme.to_str().expect("theme path")]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("valid theme"), "{stdout}");
    assert!(!stdout.contains("\"ok\""), "{stdout}");
}

#[test]
fn check_refuses_an_unknown_property_with_exit_status_one() {
    let fixture = Fixture::new();
    let theme = fixture.source_theme(GAME, "night-city", &[]);
    fs::write(
        theme.join("theme.css"),
        ".modn-card { transform: scale(2); }",
    )
    .expect("broken css");
    let output = fixture.run(&["theme", "check", theme.to_str().expect("theme path")]);
    assert_eq!(output.status.code(), Some(1));
    let value: Value = serde_json::from_slice(&output.stdout).expect("JSON output");
    assert_eq!(value["ok"], false, "{value}");
    assert_eq!(value["theme"], Value::Null);
    let diagnostics = value["diagnostics"].as_array().expect("diagnostics");
    assert_eq!(diagnostics.len(), 1, "{value}");
    assert_eq!(diagnostics[0]["rule"], "unknown-property");
    assert_eq!(diagnostics[0]["theme"], "cyberpunk-2077/night-city");
    assert!(diagnostics[0]["message"].as_str().is_some());
}

#[test]
fn register_copies_a_valid_theme_and_reports_its_digest() {
    let fixture = Fixture::new();
    let theme = fixture.source_theme(GAME, "night-city", &["neon"]);
    let value = fixture.register(&theme);
    assert_eq!(value["registered"]["id"], "night-city");
    assert_eq!(value["registered"]["game"], GAME);
    assert_eq!(value["registered"]["source"]["kind"], "local");
    assert_eq!(value["registered"]["selected"], false);
    let digest = value["digest"].as_str().expect("digest");
    assert!(digest.starts_with("blake3:"), "{digest}");
    assert_eq!(digest.len(), "blake3:".len() + 64);
    assert!(fixture
        .registry_entry(GAME, "night-city")
        .join("theme")
        .join("theme.json")
        .is_file());
    assert!(theme.join("theme.json").is_file());
}

#[test]
fn register_refuses_an_invalid_theme_and_copies_nothing() {
    let fixture = Fixture::new();
    let theme = fixture.source_theme(GAME, "night-city", &[]);
    fs::write(theme.join("theme.css"), ".modn-app { transform: none; }").expect("css");
    let output = fixture.failed(&["theme", "register", theme.to_str().expect("theme path")]);
    assert_eq!(output.status.code(), Some(1));
    let value: Value = serde_json::from_slice(&output.stderr).expect("refusal envelope");
    assert_eq!(value["error"], "THEME_INVALID");
    assert!(value["message"]
        .as_str()
        .is_some_and(|text| !text.is_empty()));
    let diagnostics = value["diagnostics"].as_array().expect("diagnostics");
    assert!(!diagnostics.is_empty());
    assert_eq!(diagnostics[0]["rule"], "unknown-property");
    assert!(!fixture.registry_entry(GAME, "night-city").exists());
    assert!(!fixture.state.join("themes").exists());
}

#[test]
fn the_staged_example_passes_check_and_register() {
    let fixture = Fixture::new();
    let example = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/themes/cyberpunk-2077/night-city");
    let checked = fixture.json(&["theme", "check", example.to_str().expect("example path")]);
    assert_eq!(checked["ok"], true, "{checked}");
    assert_eq!(checked["theme"]["id"], "night-city");
    assert_eq!(
        checked["theme"]["variants"],
        serde_json::json!(["neon", "noir"])
    );
    assert_eq!(checked["theme"]["icon"], "assets/icon.png");
    let registered = fixture.register(&example);
    assert_eq!(registered["registered"]["id"], "night-city");
    assert_eq!(registered["registered"]["name"], "Night City");
    assert!(registered["digest"]
        .as_str()
        .is_some_and(|digest| digest.starts_with("blake3:")));
    assert_eq!(
        fs::read_to_string(
            fixture
                .registry_entry("cyberpunk-2077", "night-city")
                .join("theme")
                .join("theme.css")
        )
        .expect("copied css"),
        fs::read_to_string(example.join("theme.css")).expect("source css")
    );
}
