//! Inspection of one theme directory.

mod assets;
mod entries;
mod reading;

use super::css::{CheckedCss, CssChecker, CssContext};
use super::digest::theme_digest;
use super::limits::{
    MAX_THEME_ASSETS, MAX_THEME_ASSETS_BYTES, MAX_THEME_CSS_BYTES, MAX_THEME_MANIFEST_BYTES,
    MAX_THEME_VARIANT_CSS_BYTES,
};
use super::manifest::{ThemeManifest, MANIFEST_FILE};
use super::problem::{Rule, ThemeProblem};
use assets::{scan_assets, AssetScan};
pub use assets::{AssetInfo, AssetKind};
use entries::{inspect_entry, TopLevel};
use reading::{read_bounded, ReadFailure};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// The base stylesheet name inside a theme directory.
const CSS_FILE: &str = "theme.css";
/// The directory that holds theme assets.
const ASSETS_DIR: &str = "assets";
/// The most directory entries one inspection visits in the theme directory.
const MAX_THEME_ENTRIES: usize = 1024;

/// Counts the directory entries visited while one theme is inspected.
#[derive(Debug)]
pub(super) struct EntryBudget {
    limit: usize,
    remaining: usize,
    refused: bool,
}

impl EntryBudget {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            remaining: limit,
            refused: false,
        }
    }

    /// Take one entry from the budget; return false when it is exhausted.
    pub(super) fn take(&mut self) -> bool {
        if self.remaining == 0 {
            false
        } else {
            self.remaining -= 1;
            true
        }
    }

    /// Return the single refusal for an exhausted budget.
    pub(super) fn refusal(&mut self) -> Option<ThemeProblem> {
        if self.refused {
            return None;
        }
        self.refused = true;
        Some(
            ThemeProblem::new(
                Rule::ThemeTooLarge,
                format!(
                    "the theme directory holds more than {} entries; scanning stopped",
                    self.limit
                ),
            )
            .with_path("."),
        )
    }

    #[cfg(test)]
    pub(super) fn visited(&self) -> usize {
        self.limit - self.remaining
    }
}

/// A theme directory that passed every check.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectedTheme {
    /// Validated `theme.json`.
    pub manifest: ThemeManifest,
    /// The inspected directory, as passed to [`inspect_theme_dir`].
    pub root: PathBuf,
    /// Canonical base stylesheet from `theme.css`.
    pub css: String,
    /// Canonical stylesheet for each variant id.
    pub variant_css: BTreeMap<String, String>,
    /// Every valid regular file under `assets/`, sorted by path.
    pub assets: Vec<AssetInfo>,
    /// BLAKE3 digest of the whole directory.
    pub digest: String,
}

/// Inspect one theme directory and collect every problem it has.
///
/// The function never follows links, never reads a file beyond its limit plus
/// one byte and never panics on hostile input. When at least one problem is
/// found it returns all of them instead of a theme.
///
/// # Panics
///
/// Panics only if an internal invariant fails after every check passed. No
/// input, hostile or otherwise, can reach that state.
pub fn inspect_theme_dir(
    dir: &Path,
    checker: &dyn CssChecker,
) -> Result<InspectedTheme, Vec<ThemeProblem>> {
    let mut problems = Vec::new();
    match fs::symlink_metadata(dir) {
        Err(error) => {
            problems.push(
                ThemeProblem::new(
                    Rule::ManifestInvalid,
                    format!("theme directory is missing or unreadable: {error}"),
                )
                .with_path(MANIFEST_FILE),
            );
            return Err(problems);
        }
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(vec![ThemeProblem::new(
                Rule::UnexpectedFile,
                "the theme directory is a link; links are refused",
            )
            .with_path(".")]);
        }
        Ok(metadata) if !metadata.is_dir() => {
            problems.push(
                ThemeProblem::new(Rule::ManifestInvalid, "the theme path is not a directory")
                    .with_path(MANIFEST_FILE),
            );
            return Err(problems);
        }
        Ok(_) => {}
    }

    let manifest = read_manifest(dir, &mut problems);
    let variants = manifest
        .as_ref()
        .map_or_else(Vec::new, |manifest| manifest.variants.clone());

    let mut top = TopLevel::default();
    let mut budget = EntryBudget::new(MAX_THEME_ENTRIES);
    match fs::read_dir(dir) {
        Ok(entries) => {
            for entry in entries {
                if !budget.take() {
                    if let Some(problem) = budget.refusal() {
                        problems.push(problem);
                    }
                    break;
                }
                match entry {
                    Ok(entry) => inspect_entry(&entry, &variants, &mut top, &mut problems),
                    Err(error) => problems.push(
                        ThemeProblem::new(
                            Rule::UnexpectedFile,
                            format!("cannot read a theme entry: {error}"),
                        )
                        .with_path("."),
                    ),
                }
            }
        }
        Err(error) => problems.push(
            ThemeProblem::new(
                Rule::ManifestInvalid,
                format!("cannot list the theme directory: {error}"),
            )
            .with_path(MANIFEST_FILE),
        ),
    }

    if !top.seen.contains(CSS_FILE) {
        problems
            .push(ThemeProblem::new(Rule::MissingCss, "theme.css is missing").with_path(CSS_FILE));
    }
    for variant in &variants {
        let file = variant_file(variant);
        if !top.seen.contains(&file) {
            problems.push(
                ThemeProblem::new(
                    Rule::MissingVariantFile,
                    format!("variant \"{variant}\" has no {file}"),
                )
                .with_path(file),
            );
        }
    }

    let mut stylesheets = Vec::new();
    if let Some(size) = top.files.get(CSS_FILE) {
        stylesheets.push((CSS_FILE.to_owned(), *size));
    }
    for variant in &variants {
        let file = variant_file(variant);
        if let Some(size) = top.files.get(&file) {
            stylesheets.push((file, *size));
        }
    }
    for (file, size) in &stylesheets {
        let limit = css_limit(file);
        if *size > limit {
            problems.push(
                ThemeProblem::new(
                    Rule::ThemeTooLarge,
                    format!("\"{file}\" is {size} bytes; the limit is {limit}"),
                )
                .with_path(file),
            );
        }
    }

    let mut scan = if top.assets {
        scan_assets(&dir.join(ASSETS_DIR), &mut budget)
    } else {
        AssetScan::default()
    };
    problems.append(&mut scan.problems);
    if scan.count > MAX_THEME_ASSETS {
        problems.push(
            ThemeProblem::new(
                Rule::TooManyAssets,
                format!(
                    "assets/ holds {} files; the limit is {MAX_THEME_ASSETS}",
                    scan.count
                ),
            )
            .with_path(ASSETS_DIR),
        );
    }
    let total = scan.total_bytes;
    if total > MAX_THEME_ASSETS_BYTES {
        problems.push(
            ThemeProblem::new(
                Rule::ThemeTooLarge,
                format!("assets/ files total {total} bytes; the limit is {MAX_THEME_ASSETS_BYTES}"),
            )
            .with_path("."),
        );
    }
    if let Some(icon) = manifest
        .as_ref()
        .and_then(|manifest| manifest.icon.as_deref())
    {
        if !scan.present.contains(icon) {
            problems.push(
                ThemeProblem::new(
                    Rule::AssetType,
                    format!("icon \"{icon}\" is not a file under assets/"),
                )
                .with_path(icon),
            );
        }
    }

    let mut base_css = None;
    let mut variant_css = BTreeMap::new();
    for (file, size) in &stylesheets {
        if *size > css_limit(file) {
            continue;
        }
        let bytes = match read_bounded(&dir.join(file), css_limit(file)) {
            Ok(bytes) => bytes,
            Err(error) => {
                problems.push(read_problem(file, &error));
                continue;
            }
        };
        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(error) => {
                problems.push(
                    ThemeProblem::new(
                        Rule::CssSyntax,
                        format!("\"{file}\" is not valid UTF-8: {error}"),
                    )
                    .with_path(file),
                );
                continue;
            }
        };
        let variant = base_variant(file);
        let context = match &variant {
            Some(variant) => CssContext::for_variant(variant.clone()),
            None => CssContext::base(),
        };
        match checker.check(file, &text, &context) {
            Ok(checked) => {
                check_asset_urls(file, &checked, &scan.present, &mut problems);
                match variant {
                    Some(variant) => {
                        variant_css.insert(variant, checked.text);
                    }
                    None => base_css = Some(checked.text),
                }
            }
            Err(reported) => {
                for mut problem in reported {
                    if problem.path.is_none() {
                        problem.path = Some(file.clone());
                    }
                    problems.push(problem);
                }
            }
        }
    }

    let digest = if problems.is_empty() {
        match theme_digest(dir) {
            Ok(digest) => Some(digest),
            Err(problem) => {
                problems.push(problem);
                None
            }
        }
    } else {
        None
    };
    if let Some(manifest) = &manifest {
        let label = format!("{}/{}", manifest.game, manifest.id);
        for problem in &mut problems {
            if problem.theme.is_none() {
                problem.theme = Some(label.clone());
            }
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }

    let manifest = manifest.expect("a valid theme has a parsed manifest");
    let css = base_css.expect("a valid theme has canonical base css");
    let digest = digest.expect("a valid theme has a digest");
    Ok(InspectedTheme {
        manifest,
        root: dir.to_path_buf(),
        css,
        variant_css,
        assets: scan.files,
        digest,
    })
}

fn variant_file(variant: &str) -> String {
    format!("theme.{variant}.css")
}

fn base_variant(file: &str) -> Option<String> {
    if file == CSS_FILE {
        None
    } else {
        entries::variant_of_file(file).map(str::to_owned)
    }
}

fn css_limit(file: &str) -> u64 {
    if file == CSS_FILE {
        MAX_THEME_CSS_BYTES
    } else {
        MAX_THEME_VARIANT_CSS_BYTES
    }
}

fn read_manifest(dir: &Path, problems: &mut Vec<ThemeProblem>) -> Option<ThemeManifest> {
    match read_bounded(&dir.join(MANIFEST_FILE), MAX_THEME_MANIFEST_BYTES) {
        Ok(bytes) => match ThemeManifest::parse(&bytes) {
            Ok(manifest) => Some(manifest),
            Err(problem) => {
                problems.push(problem);
                None
            }
        },
        Err(ReadFailure::Link) => None,
        Err(ReadFailure::TooLarge) => {
            problems.push(
                ThemeProblem::new(
                    Rule::ManifestInvalid,
                    format!("theme.json exceeds the {MAX_THEME_MANIFEST_BYTES}-byte limit"),
                )
                .with_path(MANIFEST_FILE),
            );
            None
        }
        Err(ReadFailure::NotFile) => {
            problems.push(
                ThemeProblem::new(Rule::ManifestInvalid, "theme.json is not a regular file")
                    .with_path(MANIFEST_FILE),
            );
            None
        }
        Err(ReadFailure::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            problems.push(
                ThemeProblem::new(Rule::ManifestInvalid, "theme.json is missing")
                    .with_path(MANIFEST_FILE),
            );
            None
        }
        Err(ReadFailure::Io(error)) => {
            problems.push(
                ThemeProblem::new(
                    Rule::ManifestInvalid,
                    format!("cannot read theme.json: {error}"),
                )
                .with_path(MANIFEST_FILE),
            );
            None
        }
    }
}

fn read_problem(file: &str, error: &ReadFailure) -> ThemeProblem {
    ThemeProblem::new(
        Rule::UnexpectedFile,
        format!("cannot read \"{file}\": {error}"),
    )
    .with_path(file)
}

fn check_asset_urls(
    file: &str,
    checked: &CheckedCss,
    present: &BTreeSet<String>,
    problems: &mut Vec<ThemeProblem>,
) {
    let urls: BTreeSet<&String> = checked.asset_urls.iter().collect();
    for url in urls {
        if !present.contains(url.as_str()) {
            problems.push(
                ThemeProblem::new(
                    Rule::UrlOutsideAssets,
                    format!(
                        "\"{file}\" references url({url}), which is not an asset of this theme"
                    ),
                )
                .with_path(file),
            );
        }
    }
}

#[cfg(test)]
#[path = "inspect_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "inspect_assets_tests.rs"]
mod asset_tests;
