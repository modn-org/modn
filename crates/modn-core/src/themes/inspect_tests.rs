use crate::themes::{
    inspect_theme_dir, theme_digest, AssetKind, CheckedCss, CssChecker, CssContext, Rule,
    ThemeProblem,
};
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

const MANIFEST: &str = r#"{"schema_version":1,"id":"night-city","game":"cyberpunk-2077","name":"Night City","theme_api":1,"variants":["neon","noir"],"icon":"assets/icon.png"}"#;
const MANIFEST_WITHOUT_ICON: &str = r#"{"schema_version":1,"id":"night-city","game":"cyberpunk-2077","name":"Night City","theme_api":1,"variants":[]}"#;
const PNG: [u8; 9] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00];

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

#[derive(Default)]
struct Recording {
    calls: RefCell<Vec<(String, Option<String>)>>,
}

impl CssChecker for Recording {
    fn check(
        &self,
        file: &str,
        css: &str,
        ctx: &CssContext,
    ) -> Result<CheckedCss, Vec<ThemeProblem>> {
        self.calls
            .borrow_mut()
            .push((file.to_owned(), ctx.variant.clone()));
        Ok(CheckedCss::new(format!("canonical:{css}"), Vec::new()))
    }
}

struct Reject;

impl CssChecker for Reject {
    fn check(
        &self,
        _file: &str,
        _css: &str,
        _ctx: &CssContext,
    ) -> Result<CheckedCss, Vec<ThemeProblem>> {
        Err(vec![ThemeProblem::new(
            Rule::UnknownProperty,
            "property \"transform\" is not allowed",
        )
        .with_line(4)
        .with_path("inner.css")])
    }
}

struct WithUrls(Vec<String>);

impl CssChecker for WithUrls {
    fn check(
        &self,
        _file: &str,
        css: &str,
        _ctx: &CssContext,
    ) -> Result<CheckedCss, Vec<ThemeProblem>> {
        Ok(CheckedCss::new(css.to_owned(), self.0.clone()))
    }
}

fn write(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
}

fn build(root: &Path, manifest: &str) {
    write(&root.join("theme.json"), manifest.as_bytes());
    write(&root.join("theme.css"), b".modn-app { color: #ffffff; }");
    write(
        &root.join("theme.neon.css"),
        b".modn-app { color: #ff00ff; }",
    );
    write(
        &root.join("theme.noir.css"),
        b".modn-app { color: #000000; }",
    );
    write(&root.join("assets/icon.png"), &PNG);
}

fn valid_theme() -> (tempfile::TempDir, PathBuf) {
    let temp = tempdir().unwrap();
    let root = temp.path().join("cyberpunk-2077/night-city");
    build(&root, MANIFEST);
    (temp, root)
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
fn a_valid_theme_is_inspected() {
    let (_temp, root) = valid_theme();
    let theme = inspect_theme_dir(&root, &Accept).unwrap();
    assert_eq!(theme.manifest.id, "night-city");
    assert_eq!(theme.manifest.game, "cyberpunk-2077");
    assert_eq!(theme.root, root);
    assert_eq!(theme.css, ".modn-app { color: #ffffff; }");
    assert_eq!(theme.variant_css.len(), 2);
    assert_eq!(theme.variant_css["neon"], ".modn-app { color: #ff00ff; }");
    assert_eq!(theme.assets.len(), 1);
    assert_eq!(theme.assets[0].path, "assets/icon.png");
    assert_eq!(theme.assets[0].kind, AssetKind::Png);
    assert_eq!(theme.assets[0].bytes, PNG.len() as u64);
    assert_eq!(theme.digest.len(), 64);
}

#[test]
fn the_checker_receives_base_and_variant_context() {
    let (_temp, root) = valid_theme();
    let checker = Recording::default();
    inspect_theme_dir(&root, &checker).unwrap();
    assert_eq!(
        checker.calls.borrow().as_slice(),
        [
            ("theme.css".to_owned(), None),
            ("theme.neon.css".to_owned(), Some("neon".to_owned())),
            ("theme.noir.css".to_owned(), Some("noir".to_owned())),
        ]
    );
}

#[test]
fn canonical_css_from_the_checker_is_returned() {
    let (_temp, root) = valid_theme();
    let theme = inspect_theme_dir(&root, &Recording::default()).unwrap();
    assert!(theme.css.starts_with("canonical:"));
    assert!(theme.variant_css["noir"].starts_with("canonical:"));
}

#[test]
fn a_missing_directory_is_manifest_invalid() {
    let temp = tempdir().unwrap();
    let problems = problems(&temp.path().join("missing"));
    let problem = find(&problems, Rule::ManifestInvalid);
    assert_eq!(problem.path.as_deref(), Some("theme.json"));
    assert_eq!(problem.theme, None);
}

#[test]
fn a_missing_manifest_is_manifest_invalid() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("theme");
    write(&root.join("theme.css"), b"");
    let problems = problems(&root);
    assert_eq!(
        find(&problems, Rule::ManifestInvalid).path.as_deref(),
        Some("theme.json")
    );
}

#[test]
fn an_unknown_manifest_field_is_reported() {
    let (_temp, root) = valid_theme();
    write(
        &root.join("theme.json"),
        br#"{"schema_version":1,"id":"night-city","game":"cyberpunk-2077","name":"Night City","theme_api":1,"variants":["neon","noir"],"icon":"assets/icon.png","extra":true}"#,
    );
    let problem = find(&problems(&root), Rule::UnknownManifestField).clone();
    assert!(problem.message.contains("extra"), "{}", problem.message);
    assert_eq!(problem.path.as_deref(), Some("theme.json"));
}

#[test]
fn a_missing_base_stylesheet_is_missing_css() {
    let (_temp, root) = valid_theme();
    fs::remove_file(root.join("theme.css")).unwrap();
    let problems = problems(&root);
    let problem = find(&problems, Rule::MissingCss);
    assert_eq!(problem.path.as_deref(), Some("theme.css"));
    assert_eq!(problem.theme.as_deref(), Some("cyberpunk-2077/night-city"));
}

#[test]
fn a_missing_variant_stylesheet_is_missing_variant_file() {
    let (_temp, root) = valid_theme();
    fs::remove_file(root.join("theme.noir.css")).unwrap();
    let problem = find(&problems(&root), Rule::MissingVariantFile).clone();
    assert_eq!(problem.path.as_deref(), Some("theme.noir.css"));
    assert!(problem.message.contains("noir"));
}

#[test]
fn an_unlisted_variant_stylesheet_is_unexpected_file() {
    let (_temp, root) = valid_theme();
    write(&root.join("theme.extra.css"), b".modn-app {}");
    let problem = find(&problems(&root), Rule::UnexpectedFile).clone();
    assert_eq!(problem.path.as_deref(), Some("theme.extra.css"));
}

#[test]
fn an_unexpected_top_level_file_is_reported() {
    let (_temp, root) = valid_theme();
    write(&root.join("notes.txt"), b"hello");
    let problem = find(&problems(&root), Rule::UnexpectedFile).clone();
    assert_eq!(problem.path.as_deref(), Some("notes.txt"));
}

#[test]
fn readme_license_and_notice_are_ignored() {
    let (_temp, root) = valid_theme();
    write(&root.join("README.md"), b"docs");
    write(&root.join("LICENSE"), b"license");
    write(&root.join("NOTICE.md"), b"notice");
    assert!(inspect_theme_dir(&root, &Accept).is_ok());
}

#[test]
fn a_missing_assets_directory_is_allowed_without_an_icon() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("theme");
    write(&root.join("theme.json"), MANIFEST_WITHOUT_ICON.as_bytes());
    write(&root.join("theme.css"), b"");
    let theme = inspect_theme_dir(&root, &Accept).unwrap();
    assert!(theme.assets.is_empty());
}

#[test]
fn a_symlink_at_top_level_is_refused() {
    let (_temp, root) = valid_theme();
    if symlink_file(&root.join("theme.css"), &root.join("README.md")).is_err() {
        eprintln!("skipping link test: this platform refuses to create symlinks");
        return;
    }
    let problem = find(&problems(&root), Rule::UnexpectedFile).clone();
    assert_eq!(problem.path.as_deref(), Some("README.md"));
    assert!(problem.message.contains("link"), "{}", problem.message);
}

#[test]
fn invalid_utf8_css_is_css_syntax() {
    let (_temp, root) = valid_theme();
    write(&root.join("theme.css"), &[0xFF, 0xFE, 0x00]);
    let problem = find(&problems(&root), Rule::CssSyntax).clone();
    assert_eq!(problem.path.as_deref(), Some("theme.css"));
}

#[test]
fn an_invalid_manifest_still_collects_other_problems() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("theme");
    write(&root.join("theme.json"), b"{}");
    write(&root.join("notes.txt"), b"hello");
    let problems = problems(&root);
    for rule in [
        Rule::ManifestInvalid,
        Rule::MissingCss,
        Rule::UnexpectedFile,
    ] {
        find(&problems, rule);
    }
    assert!(problems.iter().all(|problem| problem.theme.is_none()));
}

#[test]
fn a_rejecting_checker_contributes_problems_with_location_and_theme() {
    let (_temp, root) = valid_theme();
    let problems = problems_with(&root, &Reject);
    let problem = find(&problems, Rule::UnknownProperty);
    assert_eq!(problem.path.as_deref(), Some("inner.css"));
    assert_eq!(problem.line, Some(4));
    assert_eq!(problem.theme.as_deref(), Some("cyberpunk-2077/night-city"));
}

#[test]
fn a_checker_problem_without_a_path_gets_the_file_name() {
    struct BareReject;
    impl CssChecker for BareReject {
        fn check(
            &self,
            _file: &str,
            _css: &str,
            _ctx: &CssContext,
        ) -> Result<CheckedCss, Vec<ThemeProblem>> {
            Err(vec![ThemeProblem::new(
                Rule::ForbiddenImport,
                "@import is not allowed",
            )])
        }
    }
    let (_temp, root) = valid_theme();
    let problems = problems_with(&root, &BareReject);
    let problem = find(&problems, Rule::ForbiddenImport);
    assert_eq!(problem.path.as_deref(), Some("theme.css"));
}

#[test]
fn an_asset_url_outside_assets_is_reported() {
    let (_temp, root) = valid_theme();
    let checker = WithUrls(vec!["assets/missing.png".to_owned()]);
    let problems = problems_with(&root, &checker);
    let problem = find(&problems, Rule::UrlOutsideAssets);
    assert_eq!(problem.path.as_deref(), Some("theme.css"));
    assert!(problem.message.contains("assets/missing.png"));
}

#[test]
fn an_asset_url_that_exists_is_accepted() {
    let (_temp, root) = valid_theme();
    let checker = WithUrls(vec!["assets/icon.png".to_owned()]);
    assert!(inspect_theme_dir(&root, &checker).is_ok());
}

#[test]
fn an_oversized_base_stylesheet_is_theme_too_large() {
    let (_temp, root) = valid_theme();
    sized_file(&root.join("theme.css"), 262_145);
    let problem = find(&problems(&root), Rule::ThemeTooLarge).clone();
    assert_eq!(problem.path.as_deref(), Some("theme.css"));
}

#[test]
fn an_oversized_variant_stylesheet_is_theme_too_large() {
    let (_temp, root) = valid_theme();
    sized_file(&root.join("theme.neon.css"), 131_073);
    let problem = find(&problems(&root), Rule::ThemeTooLarge).clone();
    assert_eq!(problem.path.as_deref(), Some("theme.neon.css"));
}

#[test]
fn an_oversized_manifest_is_manifest_invalid() {
    let (_temp, root) = valid_theme();
    sized_file(&root.join("theme.json"), 16_385);
    let problem = find(&problems(&root), Rule::ManifestInvalid).clone();
    assert!(problem.message.contains("16384"), "{}", problem.message);
}

#[test]
fn the_digest_matches_the_directory_digest() {
    let (_temp, root) = valid_theme();
    let theme = inspect_theme_dir(&root, &Accept).unwrap();
    assert_eq!(theme.digest, theme_digest(&root).unwrap());
}

#[test]
fn the_digest_is_stable_across_inspections() {
    let (_temp, root) = valid_theme();
    let first = inspect_theme_dir(&root, &Accept).unwrap();
    let second = inspect_theme_dir(&root, &Accept).unwrap();
    assert_eq!(first.digest, second.digest);
}

#[test]
fn every_reported_problem_carries_a_relative_path() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("theme");
    write(&root.join("theme.json"), b"{}");
    write(&root.join("theme.css"), &[0xFF, 0xFE]);
    write(&root.join("notes.txt"), b"x");
    write(&root.join("assets/A.png"), &PNG);
    let problems = problems(&root);
    assert!(!problems.is_empty());
    for problem in &problems {
        let path = problem
            .path
            .as_deref()
            .unwrap_or_else(|| panic!("problem without a path: {problem:?}"));
        assert!(!path.contains('\\'), "windows separators in {path}");
        assert!(!path.starts_with('/') && !path.contains(":/"), "{path}");
    }
}

fn problems_with(root: &Path, checker: &dyn CssChecker) -> Vec<ThemeProblem> {
    inspect_theme_dir(root, checker).expect_err("theme should be rejected")
}

fn sized_file(path: &Path, len: u64) {
    fs::File::create(path).unwrap().set_len(len).unwrap();
}

#[cfg(unix)]
fn symlink_file(original: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(original, link)
}

#[cfg(windows)]
fn symlink_file(original: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(original, link)
}
