use crate::themes::test_support::{
    make_theme, make_theme_with_css, try_symlink_dir, write, Accept, RejectMarked, GAME, OTHER_GAME,
};
use crate::themes::{scan_bucket, BucketThemeRoot, Rule, ThemeSourceKind};
use std::fs;
use tempfile::tempdir;

fn scan(root: &BucketThemeRoot) -> crate::themes::BucketScan {
    scan_bucket(root, &Accept)
}

fn find(problems: &[crate::themes::ThemeProblem], rule: Rule) -> &crate::themes::ThemeProblem {
    problems
        .iter()
        .find(|problem| problem.rule == rule)
        .unwrap_or_else(|| panic!("no {rule:?} in {problems:#?}"))
}

#[test]
fn a_valid_bucket_holds_two_games_and_three_themes() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("bucket");
    make_theme(&root, GAME, "night-city", "Night City", &[]);
    make_theme(&root, GAME, "day-city", "Day City", &[]);
    make_theme(&root, OTHER_GAME, "joker", "Joker", &[]);

    let result = scan(&BucketThemeRoot::official(&root));
    assert_eq!(result.themes.len(), 3);
    assert!(result.problems.is_empty(), "{:#?}", result.problems);
    assert!(result
        .themes
        .iter()
        .all(|theme| theme.source().kind() == ThemeSourceKind::Official));
}

#[test]
fn a_missing_themes_directory_yields_nothing() {
    let temp = tempdir().expect("tempdir");
    let result = scan(&BucketThemeRoot::official(temp.path().join("empty")));
    assert!(result.themes.is_empty());
    assert!(result.problems.is_empty());
}

#[test]
fn themes_that_is_a_file_is_a_problem() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("bucket");
    write(&root.join("themes"), b"not a directory");
    let result = scan(&BucketThemeRoot::official(&root));
    assert!(result.themes.is_empty());
    let problem = find(&result.problems, Rule::UnexpectedFile);
    assert_eq!(problem.path.as_deref(), Some("themes"));
    assert!(problem.message.contains("not a directory"));
}

#[test]
fn themes_that_is_a_link_is_a_problem() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("bucket");
    let target = temp.path().join("elsewhere");
    fs::create_dir_all(target.join("themes")).expect("create target");
    if !try_symlink_dir(&target.join("themes"), &root.join("themes")) {
        eprintln!("skipped: this platform cannot create directory links");
        return;
    }
    let result = scan(&BucketThemeRoot::official(&root));
    assert!(result.themes.is_empty());
    let problem = find(&result.problems, Rule::UnexpectedFile);
    assert!(problem.message.contains("link"));
}

#[test]
fn a_broken_stylesheet_skips_only_that_theme() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("bucket");
    make_theme_with_css(&root, GAME, "broken", "Broken", &[], "BAD");
    make_theme(&root, GAME, "good", "Good", &[]);
    let result = scan_bucket(&BucketThemeRoot::official(&root), &RejectMarked);
    assert_eq!(result.themes.len(), 1);
    assert_eq!(result.themes[0].id(), "good");
    let problem = find(&result.problems, Rule::UnknownProperty);
    assert_eq!(problem.theme.as_deref(), Some("cyberpunk-2077/broken"));
}

#[test]
fn a_manifest_game_mismatch_is_an_id_mismatch() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("bucket");
    let dir = make_theme(&root, GAME, "night-city", "Night City", &[]);
    write(
        &dir.join("theme.json"),
        br#"{"schema_version":1,"id":"night-city","game":"balatro","name":"Night City","theme_api":1,"variants":[]}"#,
    );
    let result = scan(&BucketThemeRoot::official(&root));
    assert!(result.themes.is_empty());
    let problem = find(&result.problems, Rule::IdMismatch);
    assert!(problem.message.contains("balatro"));
    assert_eq!(problem.theme.as_deref(), Some("cyberpunk-2077/night-city"));
}

#[test]
fn a_manifest_id_mismatch_is_an_id_mismatch() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("bucket");
    let dir = make_theme(&root, GAME, "night-city", "Night City", &[]);
    write(
        &dir.join("theme.json"),
        br#"{"schema_version":1,"id":"other-city","game":"cyberpunk-2077","name":"Night City","theme_api":1,"variants":[]}"#,
    );
    let result = scan(&BucketThemeRoot::official(&root));
    assert!(result.themes.is_empty());
    let problem = find(&result.problems, Rule::IdMismatch);
    assert!(problem.message.contains("other-city"));
}

#[test]
fn an_invalid_game_directory_name_is_an_id_mismatch() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("bucket");
    fs::create_dir_all(root.join("themes/Bad_Name")).expect("create game dir");
    let result = scan(&BucketThemeRoot::official(&root));
    assert!(result.themes.is_empty());
    let problem = find(&result.problems, Rule::IdMismatch);
    assert!(problem.message.contains("Bad_Name"));
}

#[test]
fn an_invalid_theme_directory_name_is_an_id_mismatch() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("bucket");
    fs::create_dir_all(root.join("themes").join(GAME).join("Bad_Id")).expect("create theme dir");
    let result = scan(&BucketThemeRoot::official(&root));
    assert!(result.themes.is_empty());
    let problem = find(&result.problems, Rule::IdMismatch);
    assert_eq!(problem.theme.as_deref(), Some("cyberpunk-2077/Bad_Id"));
}

#[test]
fn a_stray_file_under_themes_is_unexpected() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("bucket");
    make_theme(&root, GAME, "night-city", "Night City", &[]);
    write(&root.join("themes/README.md"), b"readme");
    let result = scan(&BucketThemeRoot::official(&root));
    assert_eq!(result.themes.len(), 1);
    let problem = find(&result.problems, Rule::UnexpectedFile);
    assert_eq!(problem.path.as_deref(), Some("themes/README.md"));
}

#[test]
fn a_stray_file_under_a_game_is_unexpected() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("bucket");
    make_theme(&root, GAME, "night-city", "Night City", &[]);
    write(&root.join("themes").join(GAME).join("notes.txt"), b"notes");
    let result = scan(&BucketThemeRoot::official(&root));
    assert_eq!(result.themes.len(), 1);
    let problem = find(&result.problems, Rule::UnexpectedFile);
    assert_eq!(
        problem.path.as_deref(),
        Some("themes/cyberpunk-2077/notes.txt")
    );
}

#[test]
fn a_theme_directory_link_is_refused() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("bucket");
    let target = make_theme(
        &temp.path().join("elsewhere"),
        GAME,
        "night-city",
        "Night City",
        &[],
    );
    fs::create_dir_all(root.join("themes").join(GAME)).expect("create game dir");
    if !try_symlink_dir(&target, &root.join("themes").join(GAME).join("night-city")) {
        eprintln!("skipped: this platform cannot create directory links");
        return;
    }
    let result = scan(&BucketThemeRoot::official(&root));
    assert!(result.themes.is_empty());
    let problem = find(&result.problems, Rule::UnexpectedFile);
    assert!(problem.message.contains("link"));
}

#[test]
fn more_than_sixty_four_games_are_ignored() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("bucket");
    for index in 0..65 {
        let game = format!("game-{index:03}");
        make_theme(&root, &game, "theme", "Theme", &[]);
    }
    let result = scan(&BucketThemeRoot::official(&root));
    assert_eq!(result.themes.len(), 64);
    let problem = find(&result.problems, Rule::ThemeTooLarge);
    assert!(problem.message.contains("64"));
}

#[test]
fn more_than_two_hundred_fifty_six_themes_are_ignored() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("bucket");
    for index in 0..257 {
        let id = format!("theme-{index:03}");
        make_theme(&root, GAME, &id, "Theme", &[]);
    }
    let result = scan(&BucketThemeRoot::official(&root));
    assert_eq!(result.themes.len(), 256);
    let problem = find(&result.problems, Rule::ThemeTooLarge);
    assert!(problem.message.contains("256"));
}

#[test]
fn a_failed_manifest_keeps_the_directory_label() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("bucket");
    let dir = make_theme(&root, GAME, "night-city", "Night City", &[]);
    write(&dir.join("theme.json"), b"not json");
    let result = scan(&BucketThemeRoot::official(&root));
    assert!(result.themes.is_empty());
    assert!(result
        .problems
        .iter()
        .all(|problem| problem.theme.as_deref() == Some("cyberpunk-2077/night-city")));
}

#[test]
fn a_bucket_root_labels_its_themes_with_the_bucket_id() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("bucket");
    make_theme(&root, GAME, "night-city", "Night City", &[]);
    let bucket = BucketThemeRoot::bucket("community", &root);
    assert_eq!(bucket.bucket_id(), "community");
    assert_eq!(bucket.kind(), ThemeSourceKind::Bucket);
    assert_eq!(bucket.root(), root.as_path());
    let result = scan(&bucket);
    assert_eq!(result.themes[0].source().kind(), ThemeSourceKind::Bucket);
    assert_eq!(result.themes[0].source().bucket_id(), Some("community"));
}

#[test]
fn an_official_root_labels_its_themes_as_official() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("bucket");
    make_theme(&root, GAME, "night-city", "Night City", &[]);
    let bucket = BucketThemeRoot::official(&root);
    assert_eq!(bucket.bucket_id(), "main");
    let result = scan(&bucket);
    assert_eq!(result.themes[0].source().label(), "main bucket");
}
