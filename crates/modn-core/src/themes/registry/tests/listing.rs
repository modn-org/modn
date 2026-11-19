use super::support::{digest, register, registration_path, write, GAME, ID};
use crate::themes::{Rule, ThemeRegistry};
use std::fs;
use tempfile::tempdir;

#[test]
fn list_sorts_themes_by_game_then_id() {
    let temp = tempdir().unwrap();
    let registry = ThemeRegistry::new(&temp.path().join("data"));
    register(&registry, &temp, "game-b", "theme-a", "B A");
    register(&registry, &temp, "game-a", "theme-z", "A Z");
    register(&registry, &temp, "game-a", "theme-a", "A A");
    let listed = registry.list().unwrap();
    let order: Vec<(&str, &str)> = listed
        .themes
        .iter()
        .map(|theme| (theme.game(), theme.id()))
        .collect();
    assert_eq!(
        order,
        [
            ("game-a", "theme-a"),
            ("game-a", "theme-z"),
            ("game-b", "theme-a")
        ]
    );
    assert!(listed.problems.is_empty(), "{:#?}", listed.problems);
}

#[test]
fn list_reports_a_tampered_file_as_a_digest_mismatch() {
    let temp = tempdir().unwrap();
    let registry = ThemeRegistry::new(&temp.path().join("data"));
    let tampered = register(&registry, &temp, "game-a", "alpha", "Alpha");
    let intact = register(&registry, &temp, "game-a", "beta", "Beta");
    write(
        &tampered.dir().join("theme.css"),
        b".modn-app { color: #123456; }",
    );
    let listed = registry.list().unwrap();
    assert_eq!(listed.themes.len(), 1);
    assert_eq!(listed.themes[0].id(), intact.id());
    assert_eq!(listed.themes[0].blake3(), intact.blake3());
    assert_eq!(listed.problems.len(), 1);
    assert_eq!(listed.problems[0].rule, Rule::DigestMismatch);
    assert_eq!(listed.problems[0].theme.as_deref(), Some("game-a/alpha"));
    let path = listed.problems[0]
        .path
        .as_deref()
        .expect("a digest mismatch names the copied theme");
    assert!(path.ends_with("theme"), "{path}");
}

#[test]
fn list_reports_a_missing_registration_as_manifest_invalid() {
    let temp = tempdir().unwrap();
    let registry = ThemeRegistry::new(&temp.path().join("data"));
    let theme = register(&registry, &temp, GAME, ID, "Night City");
    fs::remove_file(registration_path(&theme)).unwrap();
    let listed = registry.list().unwrap();
    assert!(listed.themes.is_empty());
    assert_eq!(listed.problems.len(), 1);
    assert_eq!(listed.problems[0].rule, Rule::ManifestInvalid);
    assert_eq!(
        listed.problems[0].theme.as_deref(),
        Some("cyberpunk-2077/night-city")
    );
    let path = listed.problems[0].path.as_deref().unwrap();
    assert!(path.ends_with("registration.json"), "{path}");
}

#[test]
fn list_reports_a_corrupt_registration_as_manifest_invalid() {
    let temp = tempdir().unwrap();
    let registry = ThemeRegistry::new(&temp.path().join("data"));
    let theme = register(&registry, &temp, GAME, ID, "Night City");
    write(&registration_path(&theme), b"{ not json");
    let listed = registry.list().unwrap();
    assert!(listed.themes.is_empty());
    assert_eq!(listed.problems.len(), 1);
    assert_eq!(listed.problems[0].rule, Rule::ManifestInvalid);
    assert!(
        listed.problems[0].message.contains("not valid"),
        "{}",
        listed.problems[0].message
    );
}

#[test]
fn list_reports_a_registration_without_its_theme_directory() {
    let temp = tempdir().unwrap();
    let registry = ThemeRegistry::new(&temp.path().join("data"));
    let theme = register(&registry, &temp, GAME, ID, "Night City");
    fs::remove_dir_all(theme.dir()).unwrap();
    let listed = registry.list().unwrap();
    assert!(listed.themes.is_empty());
    assert_eq!(listed.problems.len(), 1);
    assert_eq!(listed.problems[0].rule, Rule::ThemeMissing);
    assert_eq!(
        listed.problems[0].theme.as_deref(),
        Some("cyberpunk-2077/night-city")
    );
}

#[test]
fn list_without_a_registry_is_empty() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let registry = ThemeRegistry::new(&data);
    let listed = registry.list().unwrap();
    assert!(listed.themes.is_empty());
    assert!(listed.problems.is_empty());
    assert!(!data.exists());
}

#[test]
fn list_keeps_the_digest_of_each_listed_theme() {
    let temp = tempdir().unwrap();
    let registry = ThemeRegistry::new(&temp.path().join("data"));
    register(&registry, &temp, "game-a", "alpha", "Alpha");
    register(&registry, &temp, "game-b", "beta", "Beta");
    let listed = registry.list().unwrap();
    assert_eq!(listed.themes.len(), 2);
    for theme in &listed.themes {
        assert_eq!(digest(theme.dir()), theme.blake3());
        assert_eq!(
            theme.name(),
            if theme.id() == "alpha" {
                "Alpha"
            } else {
                "Beta"
            }
        );
        assert_eq!(theme.variants(), &["neon".to_owned()]);
        assert_eq!(theme.icon(), Some("assets/icon.png"));
    }
}
