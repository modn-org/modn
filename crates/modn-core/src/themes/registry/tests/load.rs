use super::support::{digest, fixture, registration_path, write, Accept, BASE_CSS, GAME, ID};
use crate::themes::{Rule, ThemeError};

#[test]
fn load_returns_the_inspected_copy() {
    let (_temp, registry, source) = fixture();
    let theme = registry.register(&source, &Accept).unwrap();
    let loaded = registry.load(GAME, ID, &Accept).unwrap();
    assert_eq!(loaded.root.as_path(), theme.dir());
    assert_eq!(loaded.css, BASE_CSS);
    assert_eq!(loaded.variant_css.len(), 1);
    assert_eq!(loaded.manifest.id, ID);
    assert_eq!(loaded.manifest.game, GAME);
    assert_eq!(loaded.digest, theme.blake3());
    assert_eq!(digest(theme.dir()), loaded.digest);
}

#[test]
fn load_verifies_the_stored_digest() {
    let (_temp, registry, source) = fixture();
    let theme = registry.register(&source, &Accept).unwrap();
    write(
        &theme.dir().join("theme.css"),
        b".modn-app { color: #abcdef; }",
    );
    match registry.load(GAME, ID, &Accept) {
        Err(ThemeError::Invalid(problems)) => {
            assert_eq!(problems.len(), 1);
            assert_eq!(problems[0].rule, Rule::DigestMismatch);
            assert_eq!(
                problems[0].theme.as_deref(),
                Some("cyberpunk-2077/night-city")
            );
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn load_refuses_a_corrupt_registration() {
    let (_temp, registry, source) = fixture();
    let theme = registry.register(&source, &Accept).unwrap();
    write(&registration_path(&theme), b"{ not json");
    assert!(matches!(
        registry.load(GAME, ID, &Accept),
        Err(ThemeError::Corrupt { .. })
    ));
}

#[test]
fn load_refuses_a_registration_for_another_game() {
    let (temp, registry, source) = fixture();
    let theme = registry.register(&source, &Accept).unwrap();
    let record = serde_json::json!({
        "schema_version": 1,
        "game": "other-game",
        "id": ID,
        "blake3": theme.blake3(),
        "registered_at": theme.registered_at(),
    });
    write(
        &registration_path(&theme),
        &serde_json::to_vec(&record).unwrap(),
    );
    assert!(matches!(
        registry.load(GAME, ID, &Accept),
        Err(ThemeError::Corrupt { .. })
    ));
    assert!(temp.path().join("data/themes/registry").is_dir());
}
