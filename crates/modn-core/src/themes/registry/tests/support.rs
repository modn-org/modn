use crate::themes::{
    theme_digest, CheckedCss, CssChecker, CssContext, RegisteredTheme, Rule, ThemeProblem,
    ThemeRegistry,
};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};

pub(super) const GAME: &str = "cyberpunk-2077";
pub(super) const ID: &str = "night-city";
pub(super) const BASE_CSS: &str = ".modn-app { color: #ffffff; }";
pub(super) const PNG: [u8; 9] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00];

pub(super) struct Accept;

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

pub(super) struct Reject;

impl CssChecker for Reject {
    fn check(
        &self,
        file: &str,
        _css: &str,
        _ctx: &CssContext,
    ) -> Result<CheckedCss, Vec<ThemeProblem>> {
        Err(vec![ThemeProblem::new(
            Rule::ForbiddenImportant,
            "!important is not allowed",
        )
        .with_path(file)
        .with_line(3)])
    }
}

pub(super) fn write(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
}

pub(super) fn manifest(game: &str, id: &str, name: &str) -> String {
    format!(
        r#"{{"schema_version":1,"id":"{id}","game":"{game}","name":"{name}","theme_api":1,"variants":["neon"],"icon":"assets/icon.png"}}"#
    )
}

pub(super) fn bare_manifest(game: &str, id: &str, name: &str) -> String {
    format!(
        r#"{{"schema_version":1,"id":"{id}","game":"{game}","name":"{name}","theme_api":1,"variants":[]}}"#
    )
}

pub(super) fn make_theme(root: &Path, game: &str, id: &str, name: &str, css: &str) {
    write(
        &root.join("theme.json"),
        manifest(game, id, name).as_bytes(),
    );
    write(&root.join("theme.css"), css.as_bytes());
    write(
        &root.join("theme.neon.css"),
        b".modn-app { color: #ff00ff; }",
    );
    write(&root.join("assets/icon.png"), &PNG);
}

pub(super) fn make_bare_theme(root: &Path, game: &str, id: &str, name: &str, css: &str) {
    write(
        &root.join("theme.json"),
        bare_manifest(game, id, name).as_bytes(),
    );
    write(&root.join("theme.css"), css.as_bytes());
}

pub(super) fn fixture() -> (TempDir, ThemeRegistry, PathBuf) {
    let temp = tempdir().unwrap();
    let registry = ThemeRegistry::new(&temp.path().join("data"));
    let source = theme_source(&temp, GAME, ID, "Night City");
    (temp, registry, source)
}

pub(super) fn theme_source(temp: &TempDir, game: &str, id: &str, name: &str) -> PathBuf {
    let root = temp.path().join("sources").join(game).join(id);
    make_theme(&root, game, id, name, BASE_CSS);
    root
}

pub(super) fn register(
    registry: &ThemeRegistry,
    temp: &TempDir,
    game: &str,
    id: &str,
    name: &str,
) -> RegisteredTheme {
    let source = theme_source(temp, game, id, name);
    registry.register(&source, &Accept).unwrap()
}

pub(super) fn registration_path(theme: &RegisteredTheme) -> PathBuf {
    theme
        .dir()
        .parent()
        .expect("a theme copy has a registration parent")
        .join("registration.json")
}

pub(super) fn digest(dir: &Path) -> String {
    theme_digest(dir).unwrap()
}
