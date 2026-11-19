//! Shared fixtures for the theme catalog tests.

use crate::themes::{CheckedCss, CssChecker, CssContext, Rule, ThemeProblem, ThemeRegistry};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) const GAME: &str = "cyberpunk-2077";
pub(crate) const OTHER_GAME: &str = "balatro";

/// Accept every stylesheet.
pub(crate) struct Accept;

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

/// Reject only stylesheets that contain `BAD`, as one broken theme does.
pub(crate) struct RejectMarked;

impl CssChecker for RejectMarked {
    fn check(
        &self,
        file: &str,
        css: &str,
        _ctx: &CssContext,
    ) -> Result<CheckedCss, Vec<ThemeProblem>> {
        if css.contains("BAD") {
            return Err(vec![ThemeProblem::new(
                Rule::UnknownProperty,
                "property \"transform\" is not allowed",
            )
            .with_path(file)]);
        }
        Ok(CheckedCss::new(css.to_owned(), Vec::new()))
    }
}

/// Write one file, creating its parent directories.
pub(crate) fn write(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent directory");
    }
    fs::write(path, bytes).expect("write file");
}

/// Build one `theme.json` text.
pub(crate) fn manifest(game: &str, id: &str, name: &str, variants: &[&str]) -> String {
    let variants = variants
        .iter()
        .map(|variant| format!("\"{variant}\""))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"schema_version":1,"id":"{id}","game":"{game}","name":"{name}","theme_api":1,"variants":[{variants}]}}"#
    )
}

/// The default base stylesheet.
pub(crate) const BASE_CSS: &str = ".modn-app { color: #ffffff; }";

/// Return `<root>/themes/<game>/<id>`.
pub(crate) fn theme_dir(root: &Path, game: &str, id: &str) -> PathBuf {
    root.join("themes").join(game).join(id)
}

/// Write one valid theme under `<root>/themes/`.
pub(crate) fn make_theme(
    root: &Path,
    game: &str,
    id: &str,
    name: &str,
    variants: &[&str],
) -> PathBuf {
    make_theme_with_css(root, game, id, name, variants, BASE_CSS)
}

/// Write one theme with the given base stylesheet.
pub(crate) fn make_theme_with_css(
    root: &Path,
    game: &str,
    id: &str,
    name: &str,
    variants: &[&str],
    css: &str,
) -> PathBuf {
    let dir = theme_dir(root, game, id);
    write(
        &dir.join("theme.json"),
        manifest(game, id, name, variants).as_bytes(),
    );
    write(&dir.join("theme.css"), css.as_bytes());
    for variant in variants {
        write(
            &dir.join(format!("theme.{variant}.css")),
            format!("/* variant {variant} */").as_bytes(),
        );
    }
    dir
}

/// A registrar that writes its own source directory before copying it.
pub(crate) fn register_local(
    temp: &Path,
    game: &str,
    id: &str,
    name: &str,
    variants: &[&str],
) -> ThemeRegistry {
    let source_root = temp.join("sources");
    let source = make_theme_with_css(&source_root, game, id, name, variants, BASE_CSS);
    let registry = ThemeRegistry::new(&temp.join("data"));
    registry
        .register(&source, &Accept)
        .expect("register the local theme");
    registry
}

/// Create a directory link, reporting whether the platform allowed it.
#[cfg(unix)]
pub(crate) fn try_symlink_dir(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

/// Create a directory link, reporting whether the platform allowed it.
#[cfg(windows)]
pub(crate) fn try_symlink_dir(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_dir(target, link).is_ok()
}

/// Create a directory link, reporting whether the platform allowed it.
#[cfg(not(any(unix, windows)))]
pub(crate) fn try_symlink_dir(_target: &Path, _link: &Path) -> bool {
    false
}
