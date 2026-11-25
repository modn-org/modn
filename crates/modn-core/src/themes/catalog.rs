#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;

use super::css::CssChecker;
use super::inspect::InspectedTheme;
use super::problem::{Rule, ThemeProblem};
use super::registry::ThemeRegistry;
use super::scan::{scan_bucket, BucketThemeRoot};
use super::source::{ThemeSource, ThemeSourceKind};
use std::collections::BTreeMap;

/// One valid theme together with its source.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    inspected: InspectedTheme,
    source: ThemeSource,
}

impl CatalogEntry {
    #[must_use]
    pub(crate) fn new(inspected: InspectedTheme, source: ThemeSource) -> Self {
        Self { inspected, source }
    }

    /// The validated theme directory.
    #[must_use]
    pub fn inspected(&self) -> &InspectedTheme {
        &self.inspected
    }

    /// Where the theme came from.
    #[must_use]
    pub fn source(&self) -> &ThemeSource {
        &self.source
    }

    /// Game id the theme belongs to.
    #[must_use]
    pub fn game(&self) -> &str {
        &self.inspected.manifest.game
    }

    /// Theme id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.inspected.manifest.id
    }

    /// Display name of the theme.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.inspected.manifest.name
    }
}

/// Every valid theme from the official bucket, the local registry and trusted
/// buckets, plus every problem that skipped one.
///
/// Precedence for a duplicate `(game, id)` is official, then local, then other
/// buckets in the order given. A later duplicate is skipped with
/// `duplicate-theme-id`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeCatalog {
    entries: Vec<CatalogEntry>,
    problems: Vec<ThemeProblem>,
}

impl ThemeCatalog {
    /// Build a catalog without failing.
    ///
    /// Every broken theme, unreadable registry entry and duplicate becomes a
    /// problem. Registry I/O errors become problems too.
    #[must_use]
    pub fn build(
        roots: &[BucketThemeRoot],
        registry: &ThemeRegistry,
        checker: &dyn CssChecker,
    ) -> Self {
        let mut entries = Vec::new();
        let mut problems = Vec::new();
        for root in roots
            .iter()
            .filter(|root| root.kind() == ThemeSourceKind::Official)
        {
            append_scan(scan_bucket(root, checker), &mut entries, &mut problems);
        }
        load_registry(registry, checker, &mut entries, &mut problems);
        for root in roots
            .iter()
            .filter(|root| root.kind() != ThemeSourceKind::Official)
        {
            append_scan(scan_bucket(root, checker), &mut entries, &mut problems);
        }
        let entries = deduplicate(entries, &mut problems);
        Self { entries, problems }
    }

    /// Every valid theme, in precedence order.
    #[must_use]
    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    /// Every problem that skipped a theme or a bucket entry.
    #[must_use]
    pub fn problems(&self) -> &[ThemeProblem] {
        &self.problems
    }

    /// List themes, optionally for one game.
    ///
    /// The order is game id, then source precedence, then theme id.
    #[must_use]
    pub fn list(&self, game: Option<&str>) -> Vec<&CatalogEntry> {
        let mut entries: Vec<&CatalogEntry> = self
            .entries
            .iter()
            .filter(|entry| game.is_none_or(|game| entry.game() == game))
            .collect();
        entries.sort_by(|left, right| {
            left.game()
                .cmp(right.game())
                .then_with(|| {
                    left.source()
                        .kind()
                        .precedence()
                        .cmp(&right.source().kind().precedence())
                })
                .then_with(|| left.id().cmp(right.id()))
        });
        entries
    }

    /// Find one theme by game and id.
    #[must_use]
    pub fn get(&self, game: &str, id: &str) -> Option<&CatalogEntry> {
        self.entries
            .iter()
            .find(|entry| entry.game() == game && entry.id() == id)
    }

    pub(crate) fn problems_for_game(&self, game: &str) -> Vec<ThemeProblem> {
        let prefix = format!("{game}/");
        self.problems
            .iter()
            .filter(|problem| {
                problem
                    .theme
                    .as_deref()
                    .is_some_and(|theme| theme.starts_with(&prefix))
            })
            .cloned()
            .collect()
    }

    pub(crate) fn problems_for_theme(&self, game: &str, id: &str) -> Vec<ThemeProblem> {
        let label = format!("{game}/{id}");
        self.problems
            .iter()
            .filter(|problem| problem.theme.as_deref() == Some(label.as_str()))
            .cloned()
            .collect()
    }
}

fn append_scan(
    scan: super::scan::BucketScan,
    entries: &mut Vec<CatalogEntry>,
    problems: &mut Vec<ThemeProblem>,
) {
    entries.extend(scan.themes);
    problems.extend(scan.problems);
}

fn load_registry(
    registry: &ThemeRegistry,
    checker: &dyn CssChecker,
    entries: &mut Vec<CatalogEntry>,
    problems: &mut Vec<ThemeProblem>,
) {
    let listed = match registry.list() {
        Ok(listed) => listed,
        Err(error) => {
            problems.extend(error_problems(error, None));
            return;
        }
    };
    problems.extend(listed.problems);
    for registered in listed.themes {
        match registry.load(registered.game(), registered.id(), checker) {
            Ok(inspected) => entries.push(CatalogEntry::new(inspected, ThemeSource::local())),
            Err(error) => problems.extend(error_problems(
                error,
                Some((registered.game(), registered.id())),
            )),
        }
    }
}

fn error_problems(
    error: super::error::ThemeError,
    theme: Option<(&str, &str)>,
) -> Vec<ThemeProblem> {
    match error {
        super::error::ThemeError::Invalid(mut problems) => {
            label_problems(&mut problems, theme);
            problems
        }
        other => {
            let mut problem =
                ThemeProblem::new(Rule::ManifestInvalid, other.to_string()).with_path("registry");
            if let Some((game, id)) = theme {
                problem = problem.with_theme(format!("{game}/{id}"));
            }
            vec![problem]
        }
    }
}

fn label_problems(problems: &mut [ThemeProblem], theme: Option<(&str, &str)>) {
    let Some((game, id)) = theme else {
        return;
    };
    let label = format!("{game}/{id}");
    for problem in problems {
        if problem.theme.is_none() {
            problem.theme = Some(label.clone());
        }
    }
}

fn deduplicate(entries: Vec<CatalogEntry>, problems: &mut Vec<ThemeProblem>) -> Vec<CatalogEntry> {
    let mut seen: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut kept = Vec::with_capacity(entries.len());
    for entry in entries {
        let key = (entry.game().to_owned(), entry.id().to_owned());
        if let Some(earlier) = seen.get(&key) {
            problems.push(
                ThemeProblem::new(
                    Rule::DuplicateThemeId,
                    format!(
                        "theme \"{}\" for game \"{}\" is already provided by \"{earlier}\"; the copy from \"{}\" is skipped",
                        entry.id(),
                        entry.game(),
                        entry.source().label()
                    ),
                )
                .with_path(entry.source().label())
                .with_theme(format!("{}/{}", entry.game(), entry.id())),
            );
            continue;
        }
        seen.insert(key, entry.source().label().to_owned());
        kept.push(entry);
    }
    kept
}
