use super::SelectionStore;
use crate::themes::ThemeError;
use std::fs;
use std::path::PathBuf;
use tempfile::{tempdir, TempDir};

const GAME: &str = "cyberpunk-2077";
const THEME: &str = "night-city";

fn store() -> (TempDir, SelectionStore) {
    let temp = tempdir().unwrap();
    let store = SelectionStore::new(&temp.path().join("data"));
    (temp, store)
}

fn selection_path(temp: &TempDir, game: &str) -> PathBuf {
    temp.path()
        .join("data/themes/selection")
        .join(format!("{game}.json"))
}

fn write_selection(temp: &TempDir, game: &str, body: &str) {
    let path = selection_path(temp, game);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn selection_body(game: &str, theme: &str, variant: &str) -> String {
    format!(r#"{{"schema_version":1,"game":"{game}","theme":"{theme}","variant":{variant}}}"#)
}

#[test]
fn select_round_trips() {
    let (_temp, store) = store();
    let selected = store.select(GAME, THEME, Some("neon")).unwrap();
    assert_eq!(selected.game(), GAME);
    assert_eq!(selected.theme(), THEME);
    assert_eq!(selected.variant(), Some("neon"));
    let stored = store.get(GAME).unwrap().expect("the selection exists");
    assert_eq!(stored, selected);
}

#[test]
fn select_none_round_trips() {
    let (temp, store) = store();
    let selected = store.select_none(GAME).unwrap();
    assert_eq!(selected.game(), GAME);
    assert!(selected.is_none());
    assert_eq!(selected.theme_id(), None);
    assert_eq!(selected.theme(), "");
    assert_eq!(selected.variant(), None);
    let stored = store.get(GAME).unwrap().expect("the selection exists");
    assert_eq!(stored, selected);
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(selection_path(&temp, GAME)).unwrap()).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["game"], GAME);
    assert!(value["theme"].is_null());
    assert!(value["variant"].is_null());
}

#[test]
fn select_none_replaces_a_theme_choice_and_select_replaces_it() {
    let (_temp, store) = store();
    store.select(GAME, THEME, Some("neon")).unwrap();
    let none = store.select_none(GAME).unwrap();
    assert!(none.is_none());
    assert!(store.get(GAME).unwrap().unwrap().is_none());
    assert_eq!(store.list().unwrap().len(), 1);
    store.select(GAME, THEME, None).unwrap();
    let stored = store.get(GAME).unwrap().unwrap();
    assert!(!stored.is_none());
    assert_eq!(stored.theme_id(), Some(THEME));
    assert_eq!(stored.variant(), None);
}

#[test]
fn clear_and_clear_all_remove_a_none_selection() {
    let (_temp, store) = store();
    store.select_none("game-a").unwrap();
    store.select_none("game-b").unwrap();
    assert!(store.clear("game-a").unwrap());
    assert!(store.get("game-a").unwrap().is_none());
    let cleared = store.clear_all().unwrap();
    assert_eq!(cleared, ["game-b"]);
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn a_stored_theme_field_reads_as_a_choice_or_none() {
    let (temp, store) = store();
    write_selection(&temp, "old-game", &selection_body("old-game", THEME, "null"));
    let old = store.get("old-game").unwrap().expect("the old selection");
    assert!(!old.is_none());
    assert_eq!(old.theme_id(), Some(THEME));
    assert_eq!(old.theme(), THEME);
    write_selection(
        &temp,
        "none-game",
        r#"{"schema_version":1,"game":"none-game","theme":null,"variant":null}"#,
    );
    let none = store.get("none-game").unwrap().expect("the none selection");
    assert!(none.is_none());
    assert_eq!(none.theme_id(), None);
    assert_eq!(none.variant(), None);
}

#[test]
fn get_refuses_a_none_selection_with_a_variant() {
    let (temp, store) = store();
    write_selection(
        &temp,
        GAME,
        r#"{"schema_version":1,"game":"cyberpunk-2077","theme":null,"variant":"neon"}"#,
    );
    match store.get(GAME) {
        Err(ThemeError::Corrupt { reason, .. }) => {
            assert!(reason.contains("variant"), "{reason}");
        }
        other => panic!("expected Corrupt, got {other:?}"),
    }
}

#[test]
fn select_overwrites_the_earlier_choice() {
    let (_temp, store) = store();
    store.select(GAME, "old-theme", None).unwrap();
    store.select(GAME, THEME, Some("noir")).unwrap();
    let stored = store.get(GAME).unwrap().unwrap();
    assert_eq!(stored.theme(), THEME);
    assert_eq!(stored.variant(), Some("noir"));
    assert_eq!(store.list().unwrap().len(), 1);
}

#[test]
fn select_without_a_variant_round_trips() {
    let (temp, store) = store();
    store.select(GAME, THEME, None).unwrap();
    assert_eq!(store.get(GAME).unwrap().unwrap().variant(), None);
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(selection_path(&temp, GAME)).unwrap()).unwrap();
    assert!(value["variant"].is_null());
}

#[test]
fn selection_file_has_exactly_four_fields() {
    let (temp, store) = store();
    store.select(GAME, THEME, Some("neon")).unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(selection_path(&temp, GAME)).unwrap()).unwrap();
    let object = value.as_object().expect("the selection is an object");
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, ["game", "schema_version", "theme", "variant"]);
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["game"], GAME);
    assert_eq!(value["theme"], THEME);
    assert_eq!(value["variant"], "neon");
}

#[test]
fn select_creates_the_selection_directory_and_lock() {
    let (temp, store) = store();
    store.select(GAME, THEME, None).unwrap();
    assert!(temp.path().join("data/themes/selection").is_dir());
    assert!(temp.path().join("data/themes/themes.lock").is_file());
}

#[test]
fn select_does_not_require_a_registered_theme() {
    let (_temp, store) = store();
    store.select("game-a", "never-registered", None).unwrap();
    assert_eq!(
        store.get("game-a").unwrap().unwrap().theme(),
        "never-registered"
    );
}

#[test]
fn get_without_a_file_is_none() {
    let (_temp, store) = store();
    assert!(store.get(GAME).unwrap().is_none());
}

#[test]
fn get_refuses_bad_json() {
    let (temp, store) = store();
    write_selection(&temp, GAME, "{ not json");
    match store.get(GAME) {
        Err(ThemeError::Corrupt { path, reason }) => {
            assert!(path.ends_with(format!("{GAME}.json")));
            assert!(reason.contains("not valid"), "{reason}");
        }
        other => panic!("expected Corrupt, got {other:?}"),
    }
}

#[test]
fn get_refuses_an_unknown_field() {
    let (temp, store) = store();
    write_selection(
        &temp,
        GAME,
        r#"{"schema_version":1,"game":"cyberpunk-2077","theme":"night-city","variant":null,"extra":true}"#,
    );
    match store.get(GAME) {
        Err(ThemeError::Corrupt { reason, .. }) => {
            assert!(reason.contains("extra"), "{reason}");
        }
        other => panic!("expected Corrupt, got {other:?}"),
    }
}

#[test]
fn get_refuses_another_game() {
    let (temp, store) = store();
    write_selection(&temp, GAME, &selection_body("other-game", THEME, "null"));
    match store.get(GAME) {
        Err(ThemeError::Corrupt { reason, .. }) => {
            assert!(reason.contains("other-game"), "{reason}");
        }
        other => panic!("expected Corrupt, got {other:?}"),
    }
}

#[test]
fn get_refuses_an_invalid_theme_id() {
    let (temp, store) = store();
    write_selection(&temp, GAME, &selection_body(GAME, "Night-City", "null"));
    match store.get(GAME) {
        Err(ThemeError::Corrupt { reason, .. }) => {
            assert!(reason.contains("theme"), "{reason}");
        }
        other => panic!("expected Corrupt, got {other:?}"),
    }
}

#[test]
fn get_refuses_an_invalid_variant() {
    let (temp, store) = store();
    write_selection(&temp, GAME, &selection_body(GAME, THEME, "\"\""));
    match store.get(GAME) {
        Err(ThemeError::Corrupt { reason, .. }) => {
            assert!(reason.contains("variant"), "{reason}");
        }
        other => panic!("expected Corrupt, got {other:?}"),
    }
}

#[test]
fn get_refuses_a_wrong_schema_version() {
    let (temp, store) = store();
    write_selection(
        &temp,
        GAME,
        r#"{"schema_version":2,"game":"cyberpunk-2077","theme":"night-city","variant":null}"#,
    );
    match store.get(GAME) {
        Err(ThemeError::Corrupt { reason, .. }) => {
            assert!(reason.contains("schema_version"), "{reason}");
        }
        other => panic!("expected Corrupt, got {other:?}"),
    }
}

#[test]
fn clear_reports_whether_a_file_existed() {
    let (_temp, store) = store();
    assert!(!store.clear(GAME).unwrap());
    store.select(GAME, THEME, None).unwrap();
    assert!(store.clear(GAME).unwrap());
    assert!(store.get(GAME).unwrap().is_none());
    assert!(!store.clear(GAME).unwrap());
}

#[test]
fn clear_all_returns_sorted_games() {
    let (_temp, store) = store();
    store.select("game-b", THEME, None).unwrap();
    store.select("game-a", THEME, None).unwrap();
    store.select("game-c", THEME, Some("neon")).unwrap();
    let cleared = store.clear_all().unwrap();
    assert_eq!(cleared, ["game-a", "game-b", "game-c"]);
    assert!(store.list().unwrap().is_empty());
    assert!(store.get("game-a").unwrap().is_none());
    assert!(store.get("game-b").unwrap().is_none());
    assert!(store.get("game-c").unwrap().is_none());
}

#[test]
fn clear_all_without_files_is_empty() {
    let (_temp, store) = store();
    assert!(store.clear_all().unwrap().is_empty());
}

#[test]
fn clear_all_removes_a_corrupt_file() {
    let (temp, store) = store();
    write_selection(&temp, GAME, "{ not json");
    assert_eq!(store.clear_all().unwrap(), [GAME]);
    assert!(!selection_path(&temp, GAME).exists());
}

#[test]
fn clear_all_ignores_files_that_are_not_selections() {
    let (temp, store) = store();
    let notes = temp.path().join("data/themes/selection/notes.txt");
    fs::create_dir_all(notes.parent().unwrap()).unwrap();
    fs::write(&notes, b"keep me").unwrap();
    assert!(store.clear_all().unwrap().is_empty());
    assert!(notes.is_file());
}

#[test]
fn list_returns_sorted_selections() {
    let (_temp, store) = store();
    store.select("game-b", "theme-b", None).unwrap();
    store.select("game-a", "theme-a", Some("neon")).unwrap();
    let listed = store.list().unwrap();
    let order: Vec<&str> = listed.iter().map(|selection| selection.game()).collect();
    assert_eq!(order, ["game-a", "game-b"]);
    assert_eq!(listed[0].theme(), "theme-a");
    assert_eq!(listed[0].variant(), Some("neon"));
}

#[test]
fn list_without_directory_is_empty() {
    let (temp, store) = store();
    assert!(store.list().unwrap().is_empty());
    assert!(!temp.path().join("data").exists());
}

#[test]
fn list_refuses_a_corrupt_file() {
    let (temp, store) = store();
    store.select("game-a", THEME, None).unwrap();
    write_selection(&temp, "game-b", "{ not json");
    assert!(matches!(
        store.list(),
        Err(ThemeError::Corrupt { path, .. }) if path.ends_with("game-b.json")
    ));
}

#[test]
fn select_and_clear_reject_invalid_ids() {
    let (_temp, store) = store();
    for bad in ["", "..", "a/b", "a\\b", "Upper", "two words"] {
        assert!(
            matches!(store.select(bad, THEME, None), Err(ThemeError::InvalidId(value)) if value == bad),
            "select accepted game {bad:?}"
        );
        assert!(
            matches!(store.select(GAME, bad, None), Err(ThemeError::InvalidId(value)) if value == bad),
            "select accepted theme {bad:?}"
        );
        assert!(
            matches!(store.select(GAME, THEME, Some(bad)), Err(ThemeError::InvalidId(value)) if value == bad),
            "select accepted variant {bad:?}"
        );
        assert!(
            matches!(store.select_none(bad), Err(ThemeError::InvalidId(value)) if value == bad),
            "select_none accepted game {bad:?}"
        );
        assert!(
            matches!(store.get(bad), Err(ThemeError::InvalidId(value)) if value == bad),
            "get accepted {bad:?}"
        );
        assert!(
            matches!(store.clear(bad), Err(ThemeError::InvalidId(value)) if value == bad),
            "clear accepted {bad:?}"
        );
    }
}
