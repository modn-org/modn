use crate::themes::test_support::{make_theme, register_local, write, GAME, OTHER_GAME};
use crate::themes::{BucketThemeRoot, Rule, ThemeCatalog, ThemeRegistry};
use tempfile::tempdir;

fn build(roots: &[BucketThemeRoot], registry: &ThemeRegistry) -> ThemeCatalog {
    ThemeCatalog::build(roots, registry, &crate::themes::test_support::Accept)
}

#[test]
fn an_empty_catalog_has_no_entries_and_no_problems() {
    let temp = tempdir().expect("tempdir");
    let registry = ThemeRegistry::new(&temp.path().join("data"));
    let catalog = build(&[], &registry);
    assert!(catalog.entries().is_empty());
    assert!(catalog.problems().is_empty());
}

#[test]
fn official_themes_win_over_local_and_bucket_themes() {
    let temp = tempdir().expect("tempdir");
    let official_root = temp.path().join("official");
    let community_root = temp.path().join("community");
    make_theme(&official_root, GAME, "night-city", "Official", &[]);
    make_theme(&community_root, GAME, "night-city", "Community", &[]);
    let registry = register_local(temp.path(), GAME, "night-city", "Local", &[]);
    let roots = [
        BucketThemeRoot::bucket("community", &community_root),
        BucketThemeRoot::official(&official_root),
    ];
    let catalog = build(&roots, &registry);
    assert_eq!(catalog.entries().len(), 1);
    let entry = catalog.get(GAME, "night-city").expect("official entry");
    assert_eq!(entry.name(), "Official");
    assert_eq!(entry.source().label(), "main bucket");
    let duplicates: Vec<_> = catalog
        .problems()
        .iter()
        .filter(|problem| problem.rule == Rule::DuplicateThemeId)
        .collect();
    assert_eq!(duplicates.len(), 2);
    let paths: Vec<_> = duplicates
        .iter()
        .map(|problem| problem.path.as_deref().expect("path"))
        .collect();
    assert!(paths.contains(&"local registry"));
    assert!(paths.contains(&"community"));
}

#[test]
fn local_themes_win_over_bucket_themes() {
    let temp = tempdir().expect("tempdir");
    let community_root = temp.path().join("community");
    make_theme(&community_root, GAME, "night-city", "Community", &[]);
    let registry = register_local(temp.path(), GAME, "night-city", "Local", &[]);
    let roots = [BucketThemeRoot::bucket("community", &community_root)];
    let catalog = build(&roots, &registry);
    let entry = catalog.get(GAME, "night-city").expect("local entry");
    assert_eq!(entry.name(), "Local");
    assert_eq!(entry.source().label(), "local registry");
    let duplicate = catalog
        .problems()
        .iter()
        .find(|problem| problem.rule == Rule::DuplicateThemeId)
        .expect("duplicate problem");
    assert_eq!(duplicate.path.as_deref(), Some("community"));
}

#[test]
fn earlier_bucket_roots_win_over_later_ones() {
    let temp = tempdir().expect("tempdir");
    let first_root = temp.path().join("first");
    let second_root = temp.path().join("second");
    make_theme(&first_root, GAME, "night-city", "First", &[]);
    make_theme(&second_root, GAME, "night-city", "Second", &[]);
    let registry = ThemeRegistry::new(&temp.path().join("data"));
    let roots = [
        BucketThemeRoot::bucket("first", &first_root),
        BucketThemeRoot::bucket("second", &second_root),
    ];
    let catalog = build(&roots, &registry);
    assert_eq!(
        catalog.get(GAME, "night-city").expect("entry").name(),
        "First"
    );
    let duplicate = catalog
        .problems()
        .iter()
        .find(|problem| problem.rule == Rule::DuplicateThemeId)
        .expect("duplicate problem");
    assert_eq!(duplicate.path.as_deref(), Some("second"));
    assert_eq!(
        duplicate.theme.as_deref(),
        Some("cyberpunk-2077/night-city")
    );
}

#[test]
fn a_community_theme_that_reuses_an_official_id_is_shadowed() {
    let temp = tempdir().expect("tempdir");
    let official_root = temp.path().join("official");
    let community_root = temp.path().join("community");
    make_theme(&official_root, GAME, "night-city", "Official", &[]);
    make_theme(&community_root, GAME, "night-city", "Community", &[]);
    let registry = ThemeRegistry::new(&temp.path().join("data"));
    let roots = [
        BucketThemeRoot::official(&official_root),
        BucketThemeRoot::bucket("community", &community_root),
    ];
    let catalog = build(&roots, &registry);
    assert_eq!(catalog.entries().len(), 1);
    assert_eq!(catalog.entries()[0].name(), "Official");
    assert!(catalog
        .problems()
        .iter()
        .any(|problem| problem.rule == Rule::DuplicateThemeId));
}

#[test]
fn the_local_registry_feeds_the_catalog() {
    let temp = tempdir().expect("tempdir");
    let registry = register_local(temp.path(), GAME, "night-city", "Night City", &[]);
    let catalog = build(&[], &registry);
    assert_eq!(catalog.entries().len(), 1);
    let entry = &catalog.entries()[0];
    assert_eq!(entry.game(), GAME);
    assert_eq!(entry.id(), "night-city");
    assert_eq!(entry.name(), "Night City");
    assert_eq!(entry.source().kind(), crate::themes::ThemeSourceKind::Local);
    assert!(entry.inspected().css.contains("color"));
}

#[test]
fn a_tampered_registration_reports_a_digest_mismatch() {
    let temp = tempdir().expect("tempdir");
    let registry = register_local(temp.path(), GAME, "night-city", "Night City", &[]);
    let listed = registry.list().expect("list");
    let css = listed.themes[0].dir().join("theme.css");
    write(&css, b".modn-app { color: #000000; }");
    let catalog = build(&[], &registry);
    assert!(catalog.entries().is_empty());
    let problem = catalog
        .problems()
        .iter()
        .find(|problem| problem.rule == Rule::DigestMismatch)
        .expect("digest-mismatch problem");
    assert_eq!(problem.theme.as_deref(), Some("cyberpunk-2077/night-city"));
}

#[test]
fn a_registry_io_error_becomes_a_problem() {
    let temp = tempdir().expect("tempdir");
    write(
        &temp.path().join("data/themes/registry"),
        b"not a directory",
    );
    let registry = ThemeRegistry::new(&temp.path().join("data"));
    let catalog = build(&[], &registry);
    assert!(catalog.entries().is_empty());
    assert_eq!(catalog.problems().len(), 1);
    assert_eq!(catalog.problems()[0].rule, Rule::ManifestInvalid);
    assert!(catalog.problems()[0].message.contains("theme registry"));
}

#[test]
fn entries_cover_both_games() {
    let temp = tempdir().expect("tempdir");
    let official_root = temp.path().join("official");
    make_theme(&official_root, GAME, "night-city", "Night City", &[]);
    make_theme(&official_root, OTHER_GAME, "joker", "Joker", &[]);
    let registry = ThemeRegistry::new(&temp.path().join("data"));
    let catalog = build(&[BucketThemeRoot::official(&official_root)], &registry);
    assert_eq!(catalog.entries().len(), 2);
    assert!(catalog.get(GAME, "night-city").is_some());
    assert!(catalog.get(OTHER_GAME, "joker").is_some());
    assert!(catalog.get(GAME, "joker").is_none());
}
