use crate::themes::test_support::{
    make_theme, make_theme_with_css, register_local, theme_dir, write, Accept, RejectMarked,
    BASE_CSS, GAME, OTHER_GAME,
};
use crate::themes::{
    BucketThemeRoot, FallbackReason, ResolvedOrigin, Rule, Selection, SelectionStore, ThemeCatalog,
    ThemeRegistry, ThemeSourceKind,
};
use serde_json::json;
use std::path::Path;
use tempfile::tempdir;

fn official_catalog(temp: &Path, themes: &[(&str, &str, &[&str])]) -> ThemeCatalog {
    let root = temp.join("official");
    for (id, name, variants) in themes {
        make_theme(&root, GAME, id, name, variants);
    }
    let registry = ThemeRegistry::new(&temp.join("data"));
    ThemeCatalog::build(&[BucketThemeRoot::official(&root)], &registry, &Accept)
}

fn selection_for(temp: &Path, game: &str, theme: &str, variant: Option<&str>) -> Selection {
    SelectionStore::new(&temp.join("data"))
        .select(game, theme, variant)
        .expect("store selection")
}

#[test]
fn a_selected_theme_is_applied() {
    let temp = tempdir().expect("tempdir");
    let catalog = official_catalog(temp.path(), &[("night-city", "Night City", &[])]);
    let selection = selection_for(temp.path(), GAME, "night-city", None);
    let resolved = catalog.resolve(GAME, Some(&selection));
    let theme = resolved.theme().expect("theme");
    assert_eq!(theme.id(), "night-city");
    assert_eq!(theme.name(), "Night City");
    assert_eq!(theme.origin(), ResolvedOrigin::Selected);
    assert_eq!(theme.variant(), None);
    assert_eq!(theme.variant_css(), None);
    assert_eq!(theme.css(), BASE_CSS);
    assert_eq!(
        theme.assets_dir(),
        theme_dir(&temp.path().join("official"), GAME, "night-city").to_string_lossy()
    );
    assert!(resolved.fallback().is_none());
    assert!(resolved.diagnostics().is_empty());
}

#[test]
fn a_selected_listed_variant_is_applied() {
    let temp = tempdir().expect("tempdir");
    let catalog = official_catalog(temp.path(), &[("night-city", "Night City", &["neon"])]);
    let selection = selection_for(temp.path(), GAME, "night-city", Some("neon"));
    let resolved = catalog.resolve(GAME, Some(&selection));
    let theme = resolved.theme().expect("theme");
    assert_eq!(theme.variant(), Some("neon"));
    assert_eq!(theme.variant_css(), Some("/* variant neon */"));
    assert_eq!(theme.origin(), ResolvedOrigin::Selected);
}

#[test]
fn an_unlisted_variant_falls_back_to_the_official_theme() {
    let temp = tempdir().expect("tempdir");
    let catalog = official_catalog(
        temp.path(),
        &[
            ("night-city", "Night City", &[]),
            ("aaa-city", "AAA City", &[]),
        ],
    );
    let selection = selection_for(temp.path(), GAME, "night-city", Some("neon"));
    let resolved = catalog.resolve(GAME, Some(&selection));
    let theme = resolved.theme().expect("theme");
    assert_eq!(theme.id(), "aaa-city");
    assert_eq!(theme.origin(), ResolvedOrigin::Official);
    assert_eq!(theme.variant(), None);
    let fallback = resolved.fallback().expect("fallback");
    assert_eq!(fallback.reason(), FallbackReason::ThemeInvalid);
    assert!(fallback.message().contains("neon"));
}

#[test]
fn a_missing_selection_falls_back_to_the_official_theme() {
    let temp = tempdir().expect("tempdir");
    let catalog = official_catalog(temp.path(), &[("night-city", "Night City", &[])]);
    let selection = selection_for(temp.path(), GAME, "missing-theme", None);
    let resolved = catalog.resolve(GAME, Some(&selection));
    let theme = resolved.theme().expect("theme");
    assert_eq!(theme.id(), "night-city");
    assert_eq!(theme.origin(), ResolvedOrigin::Official);
    assert_eq!(
        resolved.fallback().expect("fallback").reason(),
        FallbackReason::ThemeMissing
    );
    assert!(resolved
        .diagnostics()
        .iter()
        .any(|problem| problem.rule == Rule::ThemeMissing
            && problem.theme.as_deref() == Some("cyberpunk-2077/missing-theme")));
}

#[test]
fn a_digest_mismatch_is_reported_and_falls_back() {
    let temp = tempdir().expect("tempdir");
    let official_root = temp.path().join("official");
    make_theme(&official_root, GAME, "aaa-city", "AAA City", &[]);
    let registry = register_local(temp.path(), GAME, "night-city", "Night City", &[]);
    let listed = registry.list().expect("list");
    write(&listed.themes[0].dir().join("theme.css"), b"changed");
    let catalog = ThemeCatalog::build(
        &[BucketThemeRoot::official(&official_root)],
        &registry,
        &Accept,
    );
    let selection = selection_for(temp.path(), GAME, "night-city", None);
    let resolved = catalog.resolve(GAME, Some(&selection));
    assert_eq!(resolved.theme().expect("theme").id(), "aaa-city");
    assert_eq!(
        resolved.fallback().expect("fallback").reason(),
        FallbackReason::DigestMismatch
    );
    assert!(resolved
        .diagnostics()
        .iter()
        .any(|problem| problem.rule == Rule::DigestMismatch));
}

#[test]
fn an_invalid_selected_theme_is_reported_and_falls_back() {
    let temp = tempdir().expect("tempdir");
    let official_root = temp.path().join("official");
    make_theme(&official_root, GAME, "aaa-city", "AAA City", &[]);
    let community_root = temp.path().join("community");
    make_theme_with_css(
        &community_root,
        GAME,
        "night-city",
        "Night City",
        &[],
        "BAD",
    );
    let registry = ThemeRegistry::new(&temp.path().join("data"));
    let catalog = ThemeCatalog::build(
        &[
            BucketThemeRoot::official(&official_root),
            BucketThemeRoot::bucket("community", &community_root),
        ],
        &registry,
        &RejectMarked,
    );
    let selection = selection_for(temp.path(), GAME, "night-city", None);
    let resolved = catalog.resolve(GAME, Some(&selection));
    assert_eq!(resolved.theme().expect("theme").id(), "aaa-city");
    assert_eq!(
        resolved.fallback().expect("fallback").reason(),
        FallbackReason::ThemeInvalid
    );
    assert!(resolved
        .diagnostics()
        .iter()
        .any(|problem| problem.rule == Rule::UnknownProperty));
}

#[test]
fn without_a_selection_the_smallest_official_id_wins() {
    let temp = tempdir().expect("tempdir");
    let catalog = official_catalog(
        temp.path(),
        &[("zeta-city", "Zeta", &[]), ("alpha-city", "Alpha", &[])],
    );
    let resolved = catalog.resolve(GAME, None);
    let theme = resolved.theme().expect("theme");
    assert_eq!(theme.id(), "alpha-city");
    assert_eq!(theme.origin(), ResolvedOrigin::Official);
    assert_eq!(theme.variant(), None);
    assert_eq!(theme.variant_css(), None);
    assert_eq!(theme.source().kind(), ThemeSourceKind::Official);
    assert!(resolved.fallback().is_none());
}

#[test]
fn a_game_without_official_themes_uses_the_default_theme() {
    let temp = tempdir().expect("tempdir");
    let catalog = official_catalog(temp.path(), &[]);
    let resolved = catalog.resolve(GAME, None);
    assert!(resolved.theme().is_none());
    assert!(resolved.fallback().is_none());
    assert!(resolved.diagnostics().is_empty());
    assert_eq!(resolved.game(), GAME);
}

#[test]
fn a_selection_for_another_game_is_ignored() {
    let temp = tempdir().expect("tempdir");
    let catalog = official_catalog(temp.path(), &[("night-city", "Night City", &[])]);
    let selection = selection_for(temp.path(), OTHER_GAME, "missing-theme", None);
    let resolved = catalog.resolve(GAME, Some(&selection));
    assert!(resolved.fallback().is_none());
    let theme = resolved.theme().expect("theme");
    assert_eq!(theme.id(), "night-city");
    assert_eq!(theme.origin(), ResolvedOrigin::Official);
}

#[test]
fn diagnostics_without_a_fallback_name_the_game() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("official");
    make_theme(&root, GAME, "night-city", "Night City", &[]);
    write(
        &root
            .join("themes")
            .join(OTHER_GAME)
            .join("broken/theme.json"),
        b"not json",
    );
    let registry = ThemeRegistry::new(&temp.path().join("data"));
    let catalog = ThemeCatalog::build(&[BucketThemeRoot::official(&root)], &registry, &Accept);
    assert!(catalog.resolve(GAME, None).diagnostics().is_empty());
    let other = catalog.resolve(OTHER_GAME, None);
    assert!(other.theme().is_none());
    assert!(!other.diagnostics().is_empty());
}

#[test]
fn a_resolved_a_selected_theme_serializes_as_documented() {
    let temp = tempdir().expect("tempdir");
    let catalog = official_catalog(temp.path(), &[("night-city", "Night City", &["neon"])]);
    let selection = selection_for(temp.path(), GAME, "night-city", Some("neon"));
    let resolved = catalog.resolve(GAME, Some(&selection));
    let assets_dir = theme_dir(&temp.path().join("official"), GAME, "night-city")
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        serde_json::to_value(&resolved).expect("serialize"),
        json!({
            "game":"cyberpunk-2077",
            "theme":{
                "id":"night-city",
                "name":"Night City",
                "source":{"kind":"official","bucket":"main","label":"main bucket"},
                "variant":"neon",
                "origin":"selected",
                "css":BASE_CSS,
                "variant_css":"/* variant neon */",
                "assets_dir":assets_dir
            },
            "fallback":null,
            "diagnostics":[]
        })
    );
}

#[test]
fn a_fallback_serializes_as_documented() {
    let temp = tempdir().expect("tempdir");
    let catalog = official_catalog(temp.path(), &[("night-city", "Night City", &[])]);
    let selection = selection_for(temp.path(), GAME, "missing-theme", None);
    let resolved = catalog.resolve(GAME, Some(&selection));
    let message =
        "the selected theme \"missing-theme\" for game \"cyberpunk-2077\" is not available";
    assert_eq!(
        serde_json::to_value(&resolved).expect("serialize"),
        json!({
            "game":"cyberpunk-2077",
            "theme":{
                "id":"night-city",
                "name":"Night City",
                "source":{"kind":"official","bucket":"main","label":"main bucket"},
                "variant":null,
                "origin":"official",
                "css":BASE_CSS,
                "variant_css":null,
                "assets_dir":theme_dir(&temp.path().join("official"), GAME, "night-city").to_string_lossy()
            },
            "fallback":{"reason":"theme-missing","message":message},
            "diagnostics":[{
                "theme":"cyberpunk-2077/missing-theme",
                "path":null,
                "line":null,
                "rule":"theme-missing",
                "message":message
            }]
        })
    );
}

#[test]
fn the_default_theme_serializes_as_documented() {
    let temp = tempdir().expect("tempdir");
    let catalog = official_catalog(temp.path(), &[]);
    assert_eq!(
        serde_json::to_value(catalog.resolve(GAME, None)).expect("serialize"),
        json!({
            "game":"cyberpunk-2077",
            "theme":null,
            "fallback":null,
            "diagnostics":[]
        })
    );
}
