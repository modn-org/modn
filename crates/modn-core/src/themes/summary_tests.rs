use crate::themes::test_support::{make_theme, register_local, write, GAME, OTHER_GAME};
use crate::themes::{
    BucketThemeRoot, SelectionStore, ThemeCatalog, ThemeRegistry, ThemeSourceKind,
};
use serde_json::json;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

fn fixture(temp: &Path) -> (ThemeCatalog, PathBuf) {
    let official_root = temp.join("official");
    make_theme(&official_root, GAME, "alpha", "Alpha", &["neon"]);
    make_theme(&official_root, GAME, "zeta", "Zeta", &[]);
    make_theme(&official_root, OTHER_GAME, "joker", "Joker", &[]);
    let community_root = temp.join("community");
    make_theme(&community_root, GAME, "community", "Community", &[]);
    let registry = register_local(temp, GAME, "beta", "Beta", &[]);
    let roots = [
        BucketThemeRoot::official(&official_root),
        BucketThemeRoot::bucket("community", &community_root),
    ];
    let catalog = ThemeCatalog::build(&roots, &registry, &crate::themes::test_support::Accept);
    (catalog, temp.join("data"))
}

fn ids(catalog: &ThemeCatalog, game: Option<&str>) -> Vec<String> {
    catalog
        .list(game)
        .iter()
        .map(|entry| entry.id().to_owned())
        .collect()
}

#[test]
fn list_orders_by_game_then_precedence_then_id() {
    let temp = tempdir().expect("tempdir");
    let (catalog, _data) = fixture(temp.path());
    assert_eq!(
        ids(&catalog, None),
        vec!["joker", "alpha", "zeta", "beta", "community"]
    );
    let kinds: Vec<ThemeSourceKind> = catalog
        .list(None)
        .iter()
        .map(|entry| entry.source().kind())
        .collect();
    assert_eq!(
        kinds,
        vec![
            ThemeSourceKind::Official,
            ThemeSourceKind::Official,
            ThemeSourceKind::Official,
            ThemeSourceKind::Local,
            ThemeSourceKind::Bucket,
        ]
    );
}

#[test]
fn list_filters_by_game() {
    let temp = tempdir().expect("tempdir");
    let (catalog, _data) = fixture(temp.path());
    assert_eq!(
        ids(&catalog, Some(GAME)),
        vec!["alpha", "zeta", "beta", "community"]
    );
    assert_eq!(ids(&catalog, Some(OTHER_GAME)), vec!["joker"]);
    assert!(ids(&catalog, Some("missing-game")).is_empty());
}

#[test]
fn a_summary_has_the_documented_shape() {
    let temp = tempdir().expect("tempdir");
    let (catalog, _data) = fixture(temp.path());
    let entry = catalog.get(GAME, "alpha").expect("alpha");
    let value = serde_json::to_value(entry.summary(true)).expect("serialize");
    assert_eq!(
        value,
        json!({
            "id":"alpha",
            "game":"cyberpunk-2077",
            "name":"Alpha",
            "theme_api":1,
            "variants":["neon"],
            "icon":null,
            "source":{"kind":"official","bucket":"main","label":"main bucket"},
            "selected":true
        })
    );
}

#[test]
fn a_summary_carries_the_icon_path() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("bucket");
    let dir = root.join("themes").join(GAME).join("icon-theme");
    write(
        &dir.join("theme.json"),
        format!(
            r#"{{"schema_version":1,"id":"icon-theme","game":"{GAME}","name":"Icon","theme_api":1,"variants":[],"icon":"assets/icon.png"}}"#
        )
        .as_bytes(),
    );
    write(&dir.join("theme.css"), b".modn-app { color: #ffffff; }");
    write(
        &dir.join("assets/icon.png"),
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
    );
    let registry = ThemeRegistry::new(&temp.path().join("data"));
    let catalog = ThemeCatalog::build(
        &[BucketThemeRoot::official(&root)],
        &registry,
        &crate::themes::test_support::Accept,
    );
    let entry = catalog.get(GAME, "icon-theme").expect("theme");
    assert_eq!(entry.summary(false).icon(), Some("assets/icon.png"));
}

#[test]
fn list_output_marks_the_stored_selection() {
    let temp = tempdir().expect("tempdir");
    let (catalog, data) = fixture(temp.path());
    let selection = SelectionStore::new(&data)
        .select(GAME, "beta", None)
        .expect("select");
    let output = catalog.list_output(None, &[selection]);
    assert_eq!(output.themes().len(), 5);
    let beta = output
        .themes()
        .iter()
        .find(|theme| theme.id() == "beta")
        .expect("beta");
    assert!(beta.selected());
    let alpha = output
        .themes()
        .iter()
        .find(|theme| theme.id() == "alpha")
        .expect("alpha");
    assert!(!alpha.selected());
}

#[test]
fn a_none_selection_marks_no_theme_selected() {
    let temp = tempdir().expect("tempdir");
    let (catalog, data) = fixture(temp.path());
    let store = SelectionStore::new(&data);
    let selection = store.select_none(GAME).expect("select none");
    let output = catalog.list_output(Some(GAME), &[selection]);
    assert!(!output.themes().is_empty());
    assert!(output.themes().iter().all(|theme| !theme.selected()));
    let selection = store.select(GAME, "alpha", None).expect("select");
    let output = catalog.list_output(Some(GAME), &[selection]);
    let selected: Vec<&str> = output
        .themes()
        .iter()
        .filter(|theme| theme.selected())
        .map(|theme| theme.id())
        .collect();
    assert_eq!(selected, ["alpha"]);
}

#[test]
fn a_selection_for_another_game_marks_nothing() {
    let temp = tempdir().expect("tempdir");
    let (catalog, data) = fixture(temp.path());
    let selection = SelectionStore::new(&data)
        .select(OTHER_GAME, "joker", None)
        .expect("select");
    let output = catalog.list_output(Some(GAME), &[selection]);
    assert!(output.themes().iter().all(|theme| !theme.selected()));
}

#[test]
fn list_output_serializes_with_themes_and_diagnostics() {
    let temp = tempdir().expect("tempdir");
    let official_root = temp.path().join("official");
    make_theme(&official_root, GAME, "alpha", "Alpha", &[]);
    let registry = ThemeRegistry::new(&temp.path().join("data"));
    let catalog = ThemeCatalog::build(
        &[BucketThemeRoot::official(&official_root)],
        &registry,
        &crate::themes::test_support::Accept,
    );
    let output = catalog.list_output(None, &[]);
    assert_eq!(
        serde_json::to_value(&output).expect("serialize"),
        json!({
            "themes":[{
                "id":"alpha",
                "game":"cyberpunk-2077",
                "name":"Alpha",
                "theme_api":1,
                "variants":[],
                "icon":null,
                "source":{"kind":"official","bucket":"main","label":"main bucket"},
                "selected":false
            }],
            "diagnostics":[]
        })
    );
    assert_eq!(output.themes().len(), 1);
    assert!(output.diagnostics().is_empty());
}

#[test]
fn list_output_filters_diagnostics_by_game() {
    let temp = tempdir().expect("tempdir");
    let official_root = temp.path().join("official");
    make_theme(&official_root, GAME, "alpha", "Alpha", &[]);
    let broken = official_root.join("themes").join(OTHER_GAME).join("broken");
    write(&broken.join("theme.json"), b"not json");
    let registry = ThemeRegistry::new(&temp.path().join("data"));
    let catalog = ThemeCatalog::build(
        &[BucketThemeRoot::official(&official_root)],
        &registry,
        &crate::themes::test_support::Accept,
    );
    let all = catalog.list_output(None, &[]);
    assert_eq!(all.diagnostics().len(), 2);
    let filtered = catalog.list_output(Some(GAME), &[]);
    assert!(filtered.diagnostics().is_empty());
    let other = catalog.list_output(Some(OTHER_GAME), &[]);
    assert_eq!(other.diagnostics().len(), 2);
}
