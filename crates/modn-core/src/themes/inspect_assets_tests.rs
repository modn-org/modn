use super::{scan_assets, EntryBudget, MAX_THEME_ENTRIES};
use crate::themes::{
    inspect_theme_dir, AssetKind, CheckedCss, CssChecker, CssContext, Rule, ThemeProblem,
};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

const MANIFEST: &str = r#"{"schema_version":1,"id":"night-city","game":"cyberpunk-2077","name":"Night City","theme_api":1,"variants":[]}"#;
const MANIFEST_WITH_ICON: &str = r#"{"schema_version":1,"id":"night-city","game":"cyberpunk-2077","name":"Night City","theme_api":1,"variants":[],"icon":"assets/icon.png"}"#;
const PNG: [u8; 9] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00];
const WEBP: [u8; 13] = *b"RIFF\0\0\0\0WEBP\0";
const WOFF2: [u8; 5] = *b"wOF2\0";

struct Accept;

impl CssChecker for Accept {
    fn check(
        &self,
        _file: &str,
        css: &str,
        _ctx: &CssContext,
    ) -> Result<CheckedCss, Vec<ThemeProblem>> {
        Ok(CheckedCss::new(css.to_owned(), Vec::new()))
    }
}

fn write(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
}

fn sized_png(path: &Path, len: u64) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut file = fs::File::create(path).unwrap();
    file.write_all(&PNG).unwrap();
    file.set_len(len).unwrap();
}

fn base(manifest: &str) -> (tempfile::TempDir, PathBuf) {
    let temp = tempdir().unwrap();
    let root = temp.path().join("cyberpunk-2077/night-city");
    write(&root.join("theme.json"), manifest.as_bytes());
    write(&root.join("theme.css"), b".modn-app {}");
    fs::create_dir_all(root.join("assets")).unwrap();
    (temp, root)
}

fn with_assets() -> (tempfile::TempDir, PathBuf) {
    base(MANIFEST)
}

fn problems(root: &Path) -> Vec<ThemeProblem> {
    inspect_theme_dir(root, &Accept).expect_err("theme should be rejected")
}

fn find(problems: &[ThemeProblem], rule: Rule) -> &ThemeProblem {
    problems
        .iter()
        .find(|problem| problem.rule == rule)
        .unwrap_or_else(|| panic!("no {rule:?} in {problems:#?}"))
}

#[test]
fn png_webp_and_woff2_assets_are_accepted() {
    let (_temp, root) = with_assets();
    write(&root.join("assets/icon.png"), &PNG);
    write(&root.join("assets/picture.webp"), &WEBP);
    write(&root.join("assets/font.woff2"), &WOFF2);
    let theme = inspect_theme_dir(&root, &Accept).unwrap();
    let kinds: Vec<AssetKind> = theme.assets.iter().map(|asset| asset.kind).collect();
    assert_eq!(kinds, [AssetKind::Woff2, AssetKind::Png, AssetKind::Webp]);
    assert_eq!(theme.assets[0].path, "assets/font.woff2");
    assert_eq!(theme.assets[2].path, "assets/picture.webp");
}

#[test]
fn a_nested_asset_directory_is_accepted() {
    let (_temp, root) = with_assets();
    write(&root.join("assets/icons/deep/a.png"), &PNG);
    let theme = inspect_theme_dir(&root, &Accept).unwrap();
    assert_eq!(theme.assets[0].path, "assets/icons/deep/a.png");
}

#[test]
fn a_missing_icon_is_asset_type() {
    let (_temp, root) = base(MANIFEST_WITH_ICON);
    let problem = find(&problems(&root), Rule::AssetType).clone();
    assert_eq!(problem.path.as_deref(), Some("assets/icon.png"));
    assert!(problem.message.contains("icon"), "{}", problem.message);
}

#[test]
fn an_unknown_extension_is_asset_type() {
    let (_temp, root) = with_assets();
    write(&root.join("assets/icon.gif"), &PNG);
    let problem = find(&problems(&root), Rule::AssetType).clone();
    assert_eq!(problem.path.as_deref(), Some("assets/icon.gif"));
}

#[test]
fn an_asset_without_an_extension_is_asset_type() {
    let (_temp, root) = with_assets();
    write(&root.join("assets/icon"), &PNG);
    let problem = find(&problems(&root), Rule::AssetType).clone();
    assert_eq!(problem.path.as_deref(), Some("assets/icon"));
}

#[test]
fn a_png_with_woff2_bytes_is_asset_type() {
    let (_temp, root) = with_assets();
    write(&root.join("assets/icon.png"), &WOFF2);
    let problem = find(&problems(&root), Rule::AssetType).clone();
    assert_eq!(problem.path.as_deref(), Some("assets/icon.png"));
    assert!(problem.message.contains("png"), "{}", problem.message);
}

#[test]
fn a_webp_with_a_bad_riff_header_is_asset_type() {
    let (_temp, root) = with_assets();
    write(&root.join("assets/picture.webp"), b"RIFF\0\0\0\0NOPE");
    let problem = find(&problems(&root), Rule::AssetType).clone();
    assert_eq!(problem.path.as_deref(), Some("assets/picture.webp"));
}

#[test]
fn an_uppercase_asset_name_is_asset_type() {
    let (_temp, root) = with_assets();
    write(&root.join("assets/Icon.png"), &PNG);
    let problem = find(&problems(&root), Rule::AssetType).clone();
    assert_eq!(problem.path.as_deref(), Some("assets/Icon.png"));
    assert!(problem.message.contains("path"), "{}", problem.message);
}

#[test]
fn an_asset_path_with_two_dots_is_asset_type() {
    let (_temp, root) = with_assets();
    write(&root.join("assets/a..b.png"), &PNG);
    let problem = find(&problems(&root), Rule::AssetType).clone();
    assert_eq!(problem.path.as_deref(), Some("assets/a..b.png"));
}

#[test]
fn an_oversized_asset_is_asset_too_large() {
    let (_temp, root) = with_assets();
    sized_png(&root.join("assets/huge.png"), 2_097_153);
    let problem = find(&problems(&root), Rule::AssetTooLarge).clone();
    assert_eq!(problem.path.as_deref(), Some("assets/huge.png"));
}

#[test]
fn sixty_five_assets_are_too_many() {
    let (_temp, root) = with_assets();
    for index in 0..65 {
        write(&root.join(format!("assets/a{index:02}.png")), &PNG);
    }
    let problem = find(&problems(&root), Rule::TooManyAssets).clone();
    assert!(problem.message.contains("65"), "{}", problem.message);
}

#[test]
fn four_full_assets_and_a_stylesheet_are_accepted() {
    let (_temp, root) = with_assets();
    for index in 0..4 {
        sized_png(&root.join(format!("assets/a{index}.png")), 2_097_152);
    }
    let theme = inspect_theme_dir(&root, &Accept).expect("assets alone are within the limit");
    assert_eq!(theme.assets.len(), 4);
}

#[test]
fn assets_over_the_total_limit_are_theme_too_large() {
    let (_temp, root) = with_assets();
    for index in 0..5 {
        sized_png(&root.join(format!("assets/a{index}.png")), 2_097_152);
    }
    let problems = problems(&root);
    let problem = find(&problems, Rule::ThemeTooLarge).clone();
    assert_eq!(problem.path.as_deref(), Some("."));
    assert!(problem.message.contains("8388608"), "{}", problem.message);
    assert!(problems
        .iter()
        .all(|problem| problem.rule != Rule::AssetTooLarge));
}

#[test]
fn two_thousand_assets_are_refused_after_an_early_stop() {
    let (_temp, root) = with_assets();
    for index in 0..2000 {
        write(&root.join(format!("assets/a{index:04}.png")), &[]);
    }
    find(&problems(&root), Rule::ThemeTooLarge);
    let mut budget = EntryBudget::new(MAX_THEME_ENTRIES);
    let scan = scan_assets(&root.join("assets"), &mut budget);
    assert!(scan
        .problems
        .iter()
        .any(|problem| problem.rule == Rule::ThemeTooLarge));
    assert_eq!(budget.visited(), MAX_THEME_ENTRIES);
}

#[test]
fn a_hundred_assets_stop_at_the_count_limit() {
    let (_temp, root) = with_assets();
    for index in 0..100 {
        write(&root.join(format!("assets/a{index:03}.png")), &[]);
    }
    find(&problems(&root), Rule::TooManyAssets);
    let mut budget = EntryBudget::new(MAX_THEME_ENTRIES);
    let scan = scan_assets(&root.join("assets"), &mut budget);
    assert_eq!(scan.count, 65, "the scan stopped at the count limit");
    assert!(
        budget.visited() <= MAX_THEME_ENTRIES,
        "visited {} entries of 100",
        budget.visited()
    );
}

#[test]
fn an_asset_tree_over_the_entry_budget_is_refused() {
    let (_temp, root) = with_assets();
    for index in 0..2000 {
        fs::create_dir(root.join(format!("assets/d{index:04}"))).unwrap();
    }
    let problem = find(&problems(&root), Rule::ThemeTooLarge).clone();
    assert_eq!(problem.path.as_deref(), Some("."));
    let mut budget = EntryBudget::new(MAX_THEME_ENTRIES);
    let scan = scan_assets(&root.join("assets"), &mut budget);
    assert!(scan
        .problems
        .iter()
        .any(|problem| problem.rule == Rule::ThemeTooLarge));
    assert_eq!(budget.visited(), MAX_THEME_ENTRIES);
}

#[test]
fn assets_must_be_a_directory() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("theme");
    write(&root.join("theme.json"), MANIFEST.as_bytes());
    write(&root.join("theme.css"), b"");
    write(&root.join("assets"), b"not a directory");
    let problem = find(&problems(&root), Rule::UnexpectedFile).clone();
    assert_eq!(problem.path.as_deref(), Some("assets"));
}

#[test]
fn a_symlink_inside_assets_is_refused() {
    let (_temp, root) = with_assets();
    write(&root.join("assets/real.png"), &PNG);
    if symlink_file(&root.join("assets/real.png"), &root.join("assets/link.png")).is_err() {
        eprintln!("skipping link test: this platform refuses to create symlinks");
        return;
    }
    let problem = find(&problems(&root), Rule::UnexpectedFile).clone();
    assert_eq!(problem.path.as_deref(), Some("assets/link.png"));
    assert!(problem.message.contains("link"), "{}", problem.message);
}

#[cfg(unix)]
fn symlink_file(original: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(original, link)
}

#[cfg(windows)]
fn symlink_file(original: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(original, link)
}
