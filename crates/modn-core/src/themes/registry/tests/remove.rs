use super::support::{digest, fixture, register, Accept, GAME, ID};
use crate::themes::{SelectionStore, ThemeError, ThemeRegistry};
use std::fs;
use tempfile::tempdir;

#[test]
fn remove_deletes_only_the_registration_and_keeps_the_source() {
    let (temp, registry, source) = fixture();
    let theme = registry.register(&source, &Accept).unwrap();
    let entry = theme.dir().parent().unwrap().to_path_buf();
    let before = digest(&source);
    let removed = registry.remove(ID, None).unwrap();
    assert_eq!(removed.game(), GAME);
    assert_eq!(removed.id(), ID);
    assert!(!entry.exists());
    assert!(source.join("theme.css").is_file());
    assert!(source.join("theme.json").is_file());
    assert_eq!(digest(&source), before);
    assert!(registry.list().unwrap().themes.is_empty());
    assert!(temp.path().join("data/themes").is_dir());
}

#[test]
fn remove_with_a_game_removes_only_that_registration() {
    let temp = tempdir().unwrap();
    let registry = ThemeRegistry::new(&temp.path().join("data"));
    register(&registry, &temp, "game-a", ID, "A");
    register(&registry, &temp, "game-b", ID, "B");
    let removed = registry.remove(ID, Some("game-a")).unwrap();
    assert_eq!(removed.game(), "game-a");
    let listed = registry.list().unwrap();
    assert_eq!(listed.themes.len(), 1);
    assert_eq!(listed.themes[0].game(), "game-b");
    assert!(listed.problems.is_empty(), "{:#?}", listed.problems);
}

#[test]
fn remove_without_a_game_refuses_an_ambiguous_id() {
    let temp = tempdir().unwrap();
    let registry = ThemeRegistry::new(&temp.path().join("data"));
    register(&registry, &temp, "game-a", ID, "A");
    register(&registry, &temp, "game-b", ID, "B");
    match registry.remove(ID, None) {
        Err(ThemeError::Ambiguous { id, games }) => {
            assert_eq!(id, ID);
            assert_eq!(games, ["game-a".to_owned(), "game-b".to_owned()]);
        }
        other => panic!("expected Ambiguous, got {other:?}"),
    }
    assert_eq!(registry.list().unwrap().themes.len(), 2);
    assert!(registry.load("game-a", ID, &Accept).is_ok());
    assert!(registry.load("game-b", ID, &Accept).is_ok());
}

#[test]
fn remove_unknown_id_is_not_registered() {
    let temp = tempdir().unwrap();
    let registry = ThemeRegistry::new(&temp.path().join("data"));
    assert!(matches!(
        registry.remove(ID, None),
        Err(ThemeError::NotRegistered { id }) if id == ID
    ));
    assert!(matches!(
        registry.remove(ID, Some(GAME)),
        Err(ThemeError::NotRegistered { .. })
    ));
    assert!(matches!(
        registry.load(GAME, ID, &Accept),
        Err(ThemeError::NotRegistered { .. })
    ));
}

#[test]
fn remove_keeps_the_selection_file() {
    let (temp, registry, source) = fixture();
    registry.register(&source, &Accept).unwrap();
    let store = SelectionStore::new(&temp.path().join("data"));
    store.select(GAME, ID, None).unwrap();
    registry.remove(ID, None).unwrap();
    let selection = store.get(GAME).unwrap().expect("the selection survives");
    assert_eq!(selection.theme(), ID);
}

#[test]
fn remove_keeps_the_other_theme_of_the_game() {
    let temp = tempdir().unwrap();
    let registry = ThemeRegistry::new(&temp.path().join("data"));
    register(&registry, &temp, GAME, "alpha", "Alpha");
    register(&registry, &temp, GAME, "beta", "Beta");
    registry.remove("alpha", Some(GAME)).unwrap();
    let listed = registry.list().unwrap();
    assert_eq!(listed.themes.len(), 1);
    assert_eq!(listed.themes[0].id(), "beta");
    assert!(registry.load(GAME, "beta", &Accept).is_ok());
    assert!(matches!(
        registry.load(GAME, "alpha", &Accept),
        Err(ThemeError::NotRegistered { .. })
    ));
    assert!(fs::symlink_metadata(
        temp.path()
            .join("data/themes/registry")
            .join(GAME)
            .join("alpha")
    )
    .is_err());
}

#[test]
fn remove_and_load_reject_invalid_ids() {
    let temp = tempdir().unwrap();
    let registry = ThemeRegistry::new(&temp.path().join("data"));
    for bad in [
        "",
        "..",
        "../x",
        "a/b",
        "a\\b",
        "Upper",
        "trailing-",
        "two words",
    ] {
        assert!(
            matches!(registry.remove(bad, None), Err(ThemeError::InvalidId(value)) if value == bad),
            "remove accepted {bad:?}"
        );
        assert!(
            matches!(registry.remove(ID, Some(bad)), Err(ThemeError::InvalidId(value)) if value == bad),
            "remove accepted game {bad:?}"
        );
        assert!(
            matches!(
                registry.load(bad, ID, &Accept),
                Err(ThemeError::InvalidId(_))
            ),
            "load accepted {bad:?}"
        );
        assert!(
            matches!(
                registry.load(GAME, bad, &Accept),
                Err(ThemeError::InvalidId(_))
            ),
            "load accepted id {bad:?}"
        );
    }
}
