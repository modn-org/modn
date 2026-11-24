use super::super::layout::Failpoint;
use super::support::{
    bare_manifest, digest, fixture, make_bare_theme, make_theme, registration_path, theme_source,
    write, Accept, Reject, BASE_CSS, GAME, ID, PNG,
};
use crate::themes::{Rule, ThemeError, ThemeRegistry};
use std::fs;
use tempfile::tempdir;

#[test]
fn register_copies_every_regular_file_and_records_the_digest() {
    let (temp, registry, source) = fixture();
    write(&source.join("README.md"), b"read me");
    write(&source.join("assets/nested/icon.png"), &PNG);
    let registered = registry.register(&source, &Accept).unwrap();
    assert_eq!(registered.game(), GAME);
    assert_eq!(registered.id(), ID);
    assert_eq!(registered.name(), "Night City");
    assert_eq!(registered.variants(), &["neon".to_owned()]);
    assert_eq!(registered.icon(), Some("assets/icon.png"));
    assert!(registered.registered_at() >= 1_600_000_000);
    assert_eq!(
        registered.dir(),
        temp.path()
            .join("data/themes/registry")
            .join(GAME)
            .join(ID)
            .join("theme")
    );
    assert_eq!(digest(&source), registered.blake3());
    assert_eq!(digest(registered.dir()), registered.blake3());
    assert_eq!(
        fs::read(registered.dir().join("theme.css")).unwrap(),
        BASE_CSS.as_bytes()
    );
    assert_eq!(
        fs::read(registered.dir().join("README.md")).unwrap(),
        b"read me"
    );
    assert_eq!(
        fs::read(registered.dir().join("assets/nested/icon.png")).unwrap(),
        PNG
    );
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(registration_path(&registered)).unwrap()).unwrap();
    let object = record.as_object().unwrap();
    assert_eq!(object.len(), 5);
    assert_eq!(record["schema_version"], 1);
    assert_eq!(record["game"], GAME);
    assert_eq!(record["id"], ID);
    assert_eq!(record["blake3"], registered.blake3());
    assert_eq!(record["registered_at"], registered.registered_at());
}

#[test]
fn register_refuses_an_invalid_theme_and_copies_nothing() {
    let (temp, registry, source) = fixture();
    let error = registry.register(&source, &Reject).unwrap_err();
    let ThemeError::Invalid(problems) = error else {
        panic!("expected Invalid, got {error:?}");
    };
    assert!(!problems.is_empty());
    assert!(problems
        .iter()
        .all(|problem| problem.rule == Rule::ForbiddenImportant));
    assert_eq!(problems[0].path.as_deref(), Some("theme.css"));
    assert!(!temp
        .path()
        .join("data/themes/registry")
        .join(GAME)
        .join(ID)
        .exists());
}

#[test]
fn register_refuses_a_manifest_with_an_invalid_id() {
    let temp = tempdir().unwrap();
    let registry = ThemeRegistry::new(&temp.path().join("data"));
    let source = temp.path().join("source");
    make_bare_theme(&source, GAME, ID, "Night City", BASE_CSS);
    write(
        &source.join("theme.json"),
        br#"{"schema_version":1,"id":"Night-City","game":"cyberpunk-2077","name":"Night City","theme_api":1,"variants":[]}"#,
    );
    let error = registry.register(&source, &Accept).unwrap_err();
    let ThemeError::Invalid(problems) = error else {
        panic!("expected Invalid, got {error:?}");
    };
    assert!(
        problems
            .iter()
            .any(|problem| problem.rule == Rule::ManifestInvalid
                && problem.message.contains("not a valid id")),
        "{problems:#?}"
    );
    assert!(!temp.path().join("data/themes/registry").join(GAME).exists());
}

#[test]
fn register_uses_the_manifest_game_and_id() {
    let temp = tempdir().unwrap();
    let registry = ThemeRegistry::new(&temp.path().join("data"));
    let source = temp.path().join("source");
    make_bare_theme(&source, GAME, ID, "Night City", BASE_CSS);
    write(
        &source.join("theme.json"),
        br#"{"schema_version":1,"id":"other-theme","game":"other-game","name":"Night City","theme_api":1,"variants":[]}"#,
    );
    let registered = registry.register(&source, &Accept).unwrap();
    assert_eq!(registered.game(), "other-game");
    assert_eq!(registered.id(), "other-theme");
    assert!(temp
        .path()
        .join("data/themes/registry/other-game/other-theme/theme/theme.css")
        .is_file());
}

#[test]
fn register_replaces_the_same_id_and_updates_the_digest() {
    let (_temp, registry, source) = fixture();
    let first = registry.register(&source, &Accept).unwrap();
    let copied = first.dir().to_path_buf();
    let old_digest = first.blake3().to_owned();
    write(&source.join("theme.css"), b".modn-app { color: #000000; }");
    let second = registry.register(&source, &Accept).unwrap();
    assert_ne!(second.blake3(), old_digest);
    assert_eq!(second.dir(), copied);
    let listed = registry.list().unwrap();
    assert_eq!(listed.themes.len(), 1);
    assert!(listed.problems.is_empty(), "{:#?}", listed.problems);
    assert_eq!(listed.themes[0].blake3(), second.blake3());
    assert_eq!(
        registry.load(GAME, ID, &Accept).unwrap().css,
        ".modn-app { color: #000000; }"
    );
}

#[test]
fn register_removes_files_that_the_new_copy_lacks() {
    let (_temp, registry, source) = fixture();
    let first = registry.register(&source, &Accept).unwrap();
    fs::remove_file(source.join("theme.neon.css")).unwrap();
    fs::remove_dir_all(source.join("assets")).unwrap();
    write(
        &source.join("theme.json"),
        bare_manifest(GAME, ID, "Night City").as_bytes(),
    );
    let second = registry.register(&source, &Accept).unwrap();
    assert_eq!(second.dir(), first.dir());
    assert!(!second.dir().join("theme.neon.css").exists());
    assert!(!second.dir().join("assets").exists());
    assert_eq!(second.variants().len(), 0);
    assert_eq!(second.icon(), None);
}

#[test]
fn register_keeps_the_same_id_for_two_games() {
    let (temp, registry, source) = fixture();
    registry.register(&source, &Accept).unwrap();
    let other = theme_source(&temp, "other-game", ID, "Night City Again");
    registry.register(&other, &Accept).unwrap();
    let listed = registry.list().unwrap();
    assert_eq!(listed.themes.len(), 2);
    assert_eq!(listed.themes[0].game(), GAME);
    assert_eq!(listed.themes[1].game(), "other-game");
    assert_eq!(listed.themes[0].id(), ID);
    assert_eq!(listed.themes[1].id(), ID);
}

#[test]
fn register_cleans_stale_staging_directories() {
    let (temp, registry, source) = fixture();
    let registry_root = temp.path().join("data/themes/registry");
    write(
        &registry_root.join(".staging-crashed/theme/theme.css"),
        b"stale",
    );
    registry.register(&source, &Accept).unwrap();
    assert!(!registry_root.join(".staging-crashed").exists());
    assert!(registry_root.join(GAME).join(ID).join("theme").is_dir());
}

#[test]
fn register_does_not_modify_the_source() {
    let (_temp, registry, source) = fixture();
    let before = digest(&source);
    registry.register(&source, &Accept).unwrap();
    assert_eq!(digest(&source), before);
    assert!(!source.join("registration.json").exists());
    assert!(source.join("theme.css").is_file());
}

#[test]
fn two_threads_register_different_themes_at_once() {
    let temp = tempdir().unwrap();
    let data = temp.path().join("data");
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    make_theme(&first, GAME, "night-city", "Night City", BASE_CSS);
    make_theme(&second, GAME, "day-city", "Day City", BASE_CSS);
    let registry = ThemeRegistry::new(&data);
    std::thread::scope(|scope| {
        let left = scope.spawn(|| registry.register(&first, &Accept));
        let right = scope.spawn(|| registry.register(&second, &Accept));
        left.join().unwrap().unwrap();
        right.join().unwrap().unwrap();
    });
    let listed = registry.list().unwrap();
    assert_eq!(listed.themes.len(), 2);
    assert!(listed.problems.is_empty(), "{:#?}", listed.problems);
    for theme in &listed.themes {
        assert_eq!(digest(theme.dir()), theme.blake3());
    }
}

#[test]
fn a_failed_replacement_restores_the_previous_copy() {
    let (_temp, registry, source) = fixture();
    let first = registry.register(&source, &Accept).unwrap();
    write(&source.join("theme.css"), b".modn-app { color: #101010; }");
    registry.fail_at(Failpoint::InstallFails);
    let error = registry.register(&source, &Accept).unwrap_err();
    assert!(matches!(error, ThemeError::Io { .. }), "{error}");
    let listed = registry.list().unwrap();
    assert_eq!(listed.themes.len(), 1);
    assert_eq!(listed.themes[0].blake3(), first.blake3());
    assert_eq!(registry.load(GAME, ID, &Accept).unwrap().css, BASE_CSS);
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(registration_path(&first)).unwrap()).unwrap();
    assert_eq!(record["blake3"], first.blake3());
    assert_eq!(record["game"], GAME);
}
