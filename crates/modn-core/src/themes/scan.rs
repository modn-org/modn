#[cfg(test)]
#[path = "scan_tests.rs"]
mod tests;

use super::catalog::CatalogEntry;
use super::css::CssChecker;
use super::inspect::inspect_theme_dir;
use super::limits::is_valid_id;
use super::problem::{Rule, ThemeProblem};
use super::source::{ThemeSource, ThemeSourceKind};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) const THEMES_DIR: &str = "themes";

const MAX_GAMES: usize = 64;

const MAX_THEMES: usize = 256;

/// One bucket root that may hold themes.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BucketThemeRoot {
    bucket: String,
    kind: ThemeSourceKind,
    root: PathBuf,
}

impl BucketThemeRoot {
    /// Use a root as the official `main` bucket.
    #[must_use]
    pub fn official(root: impl Into<PathBuf>) -> Self {
        Self {
            bucket: "main".to_owned(),
            kind: ThemeSourceKind::Official,
            root: root.into(),
        }
    }

    /// Use a root as one trusted bucket with the given id.
    #[must_use]
    pub fn bucket(id: impl Into<String>, root: impl Into<PathBuf>) -> Self {
        Self {
            bucket: id.into(),
            kind: ThemeSourceKind::Bucket,
            root: root.into(),
        }
    }

    /// Use a root with an explicit source kind.
    ///
    /// Only [`ThemeSourceKind::Official`] and [`ThemeSourceKind::Bucket`] are
    /// meaningful here. A local root is scanned as a bucket.
    #[must_use]
    pub fn new(bucket: impl Into<String>, kind: ThemeSourceKind, root: impl Into<PathBuf>) -> Self {
        Self {
            bucket: bucket.into(),
            kind,
            root: root.into(),
        }
    }

    /// Bucket id of this root.
    #[must_use]
    pub fn bucket_id(&self) -> &str {
        &self.bucket
    }

    /// Source kind of this root.
    #[must_use]
    pub fn kind(&self) -> ThemeSourceKind {
        self.kind
    }

    /// Filesystem root of the bucket.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// The themes and problems found under one bucket root.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BucketScan {
    /// Valid themes, in directory order.
    pub themes: Vec<CatalogEntry>,
    /// Every skipped entry, with its diagnostic.
    pub problems: Vec<ThemeProblem>,
}

/// Read `<root>/themes/<game>/<theme>/` and never fail.
///
/// A missing `themes/` directory yields nothing. A `themes` entry that is a
/// file or a link is one problem. Themes are capped at 64 games and 256 theme
/// directories per bucket; extra entries are ignored with one
/// `theme-too-large` problem.
#[must_use]
pub fn scan_bucket(root: &BucketThemeRoot, checker: &dyn CssChecker) -> BucketScan {
    let source = match root.kind {
        ThemeSourceKind::Official => ThemeSource::official(),
        ThemeSourceKind::Bucket | ThemeSourceKind::Local => {
            ThemeSource::bucket(root.bucket.clone())
        }
    };
    let mut scanner = Scanner {
        source: &source,
        checker,
        themes: 0,
        limit_reported: false,
        scan: BucketScan::default(),
    };
    scanner.scan_root(&root.root.join(THEMES_DIR));
    scanner.scan
}

struct Scanner<'a> {
    source: &'a ThemeSource,
    checker: &'a dyn CssChecker,
    themes: usize,
    limit_reported: bool,
    scan: BucketScan,
}

impl Scanner<'_> {
    fn scan_root(&mut self, themes_root: &Path) {
        match fs::symlink_metadata(themes_root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => {
                self.scan.problems.push(
                    ThemeProblem::new(
                        Rule::UnexpectedFile,
                        format!("cannot inspect the themes directory: {error}"),
                    )
                    .with_path(THEMES_DIR),
                );
                return;
            }
            Ok(metadata) if metadata.file_type().is_symlink() => {
                self.scan.problems.push(
                    ThemeProblem::new(
                        Rule::UnexpectedFile,
                        "the themes directory is a link; links are refused",
                    )
                    .with_path(THEMES_DIR),
                );
                return;
            }
            Ok(metadata) if !metadata.is_dir() => {
                self.scan.problems.push(
                    ThemeProblem::new(Rule::UnexpectedFile, "themes is not a directory")
                        .with_path(THEMES_DIR),
                );
                return;
            }
            Ok(_) => {}
        }
        let entries = match read_sorted(themes_root) {
            Ok(entries) => entries,
            Err(error) => {
                self.scan.problems.push(
                    ThemeProblem::new(
                        Rule::UnexpectedFile,
                        format!("cannot list the themes directory: {error}"),
                    )
                    .with_path(THEMES_DIR),
                );
                return;
            }
        };
        let mut games = 0usize;
        for entry in entries {
            let name = display_name(&entry);
            let path = format!("{THEMES_DIR}/{name}");
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    self.problem(&path, format!("cannot inspect the entry: {error}"));
                    continue;
                }
            };
            if file_type.is_symlink() {
                self.problem(&path, "the entry is a link; links are refused");
                continue;
            }
            if !file_type.is_dir() {
                self.problem(&path, "only game directories may appear under themes/");
                continue;
            }
            let Some(game) = valid_id(&name) else {
                self.scan.problems.push(
                    ThemeProblem::new(
                        Rule::IdMismatch,
                        format!("game directory name \"{name}\" is not a valid id"),
                    )
                    .with_path(path),
                );
                continue;
            };
            if games >= MAX_GAMES {
                self.report_limit(format!(
                    "the bucket has more than {MAX_GAMES} game theme directories; the rest are ignored"
                ));
                return;
            }
            games += 1;
            if !self.scan_game(&game, &entry.path()) {
                return;
            }
        }
    }

    fn scan_game(&mut self, game: &str, dir: &Path) -> bool {
        let entries = match read_sorted(dir) {
            Ok(entries) => entries,
            Err(error) => {
                self.problem(
                    &format!("{THEMES_DIR}/{game}"),
                    format!("cannot list the game theme directory: {error}"),
                );
                return true;
            }
        };
        for entry in entries {
            let name = display_name(&entry);
            let path = format!("{THEMES_DIR}/{game}/{name}");
            if self.themes >= MAX_THEMES {
                self.report_limit(format!(
                    "the bucket has more than {MAX_THEMES} theme directories; the rest are ignored"
                ));
                return false;
            }
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    self.problem(&path, format!("cannot inspect the entry: {error}"));
                    continue;
                }
            };
            if file_type.is_symlink() {
                self.problem(&path, "the theme directory is a link; links are refused");
                continue;
            }
            if !file_type.is_dir() {
                self.problem(&path, "only theme directories may appear under a game");
                continue;
            }
            let Some(id) = valid_id(&name) else {
                self.scan.problems.push(
                    ThemeProblem::new(
                        Rule::IdMismatch,
                        format!("theme directory name \"{name}\" is not a valid id"),
                    )
                    .with_theme(format!("{game}/{name}"))
                    .with_path(path),
                );
                continue;
            };
            self.themes += 1;
            self.scan_theme(game, &id, &entry.path());
        }
        true
    }

    fn scan_theme(&mut self, game: &str, id: &str, dir: &Path) {
        let label = format!("{game}/{id}");
        match inspect_theme_dir(dir, self.checker) {
            Ok(inspected) => {
                if inspected.manifest.game != game || inspected.manifest.id != id {
                    self.scan.problems.push(
                        ThemeProblem::new(
                            Rule::IdMismatch,
                            format!(
                                "theme.json names \"{}/{}\" but the directory is \"{label}\"",
                                inspected.manifest.game, inspected.manifest.id
                            ),
                        )
                        .with_theme(label)
                        .with_path("theme.json"),
                    );
                    return;
                }
                self.scan
                    .themes
                    .push(CatalogEntry::new(inspected, self.source.clone()));
            }
            Err(problems) => {
                for mut problem in problems {
                    if problem.theme.is_none() {
                        problem.theme = Some(label.clone());
                    }
                    self.scan.problems.push(problem);
                }
            }
        }
    }

    fn problem(&mut self, path: &str, message: impl Into<String>) {
        self.scan
            .problems
            .push(ThemeProblem::new(Rule::UnexpectedFile, message).with_path(path));
    }

    fn report_limit(&mut self, message: String) {
        if self.limit_reported {
            return;
        }
        self.limit_reported = true;
        self.scan
            .problems
            .push(ThemeProblem::new(Rule::ThemeTooLarge, message).with_path(THEMES_DIR));
    }
}

fn read_sorted(dir: &Path) -> std::io::Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|left| left.file_name());
    Ok(entries)
}

fn display_name(entry: &fs::DirEntry) -> String {
    entry.file_name().to_string_lossy().into_owned()
}

fn valid_id(name: &str) -> Option<String> {
    is_valid_id(name).then(|| name.to_owned())
}
