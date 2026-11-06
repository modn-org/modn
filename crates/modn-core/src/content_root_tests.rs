use super::{resolve, validate};
use crate::game_modules::ContentRootSpec;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

fn spec(path: &str) -> ContentRootSpec {
    ContentRootSpec {
        base: "appdata".into(),
        path: path.into(),
    }
}

fn rejected(game: &Path, appdata: &Path, path: &str) -> String {
    let error = resolve(game, &spec(path), appdata)
        .map(|(_, root)| root.display().to_string())
        .expect_err(path);
    format!("{error:#}")
}

#[test]
fn a_content_root_inside_the_game_root_is_rejected() {
    let temp = tempdir().unwrap();
    let game = temp.path().join("game");
    fs::create_dir_all(game.join("data/Mods")).unwrap();
    let game = game.canonicalize().unwrap();
    for (appdata, path) in [
        (game.clone(), "Mods"),
        (game.clone(), "data/Mods"),
        (game.join("data"), "Mods"),
        (game.join("data"), "Missing/Mods"),
    ] {
        let error = rejected(&game, &appdata, path);
        assert!(error.contains("overlaps the game installation"), "{error}");
    }
}

#[test]
fn a_content_root_that_contains_or_equals_the_game_root_is_rejected() {
    let temp = tempdir().unwrap();
    let appdata = temp.path().join("appdata");
    fs::create_dir_all(appdata.join("Fixture/Mods/Game")).unwrap();
    let appdata = appdata.canonicalize().unwrap();
    for (game, path) in [
        (appdata.join("Fixture/Mods/Game"), "Fixture/Mods"),
        (appdata.join("Fixture/Mods/Game"), "Fixture"),
        (appdata.join("Fixture/Mods"), "Fixture/Mods"),
    ] {
        let error = rejected(&game, &appdata, path);
        assert!(error.contains("overlaps the game installation"), "{error}");
    }
    let game = appdata.join("Fixture/Mods/Game");
    let error = validate(&game, &appdata, &appdata.join("Fixture/Mods")).unwrap_err();
    assert!(format!("{error:#}").contains("overlaps the game installation"));
}

#[test]
fn a_content_root_that_names_a_drive_or_filesystem_root_is_rejected() {
    let temp = tempdir().unwrap();
    let game = temp.path().join("game");
    let appdata = temp.path().join("appdata");
    fs::create_dir_all(&game).unwrap();
    fs::create_dir_all(&appdata).unwrap();
    let mut paths = vec!["", "/", ".", "..", "Fixture/../.."];
    if cfg!(windows) {
        paths.extend(["\\", "C:", "C:\\", "C:/", r"\\?\C:\", r"\\server\share\"]);
    }
    for path in paths {
        let error = rejected(&game, &appdata, path);
        assert!(
            error.contains("must be relative") || error.contains("parent or prefix"),
            "{path:?}: {error}"
        );
    }
    let base = appdata.canonicalize().unwrap();
    let error = validate(&game, &base, &base).unwrap_err();
    assert!(format!("{error:#}").contains("must be relative"));
    let drive = base.ancestors().last().unwrap();
    assert!(validate(&game, &base, drive).is_err());
}
