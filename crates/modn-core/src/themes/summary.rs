#[cfg(test)]
#[path = "summary_tests.rs"]
mod tests;

use super::catalog::{CatalogEntry, ThemeCatalog};
use super::problem::ThemeProblem;
use super::selection::Selection;
use super::source::ThemeSource;
use serde::Serialize;

/// One theme in the documented CLI entry shape.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThemeSummary {
    id: String,
    game: String,
    name: String,
    theme_api: u32,
    variants: Vec<String>,
    icon: Option<String>,
    source: ThemeSource,
    selected: bool,
}

impl ThemeSummary {
    /// Theme id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Game id the theme belongs to.
    #[must_use]
    pub fn game(&self) -> &str {
        &self.game
    }

    /// Display name of the theme.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Theme API version, always `1` today.
    #[must_use]
    pub fn theme_api(&self) -> u32 {
        self.theme_api
    }

    /// Variant ids listed by `theme.json`.
    #[must_use]
    pub fn variants(&self) -> &[String] {
        &self.variants
    }

    /// Icon path under `assets/`, or `None`.
    #[must_use]
    pub fn icon(&self) -> Option<&str> {
        self.icon.as_deref()
    }

    /// Where the theme came from.
    #[must_use]
    pub fn source(&self) -> &ThemeSource {
        &self.source
    }

    /// Whether this theme is the stored selection for its game.
    #[must_use]
    pub fn selected(&self) -> bool {
        self.selected
    }
}

impl CatalogEntry {
    /// Build the CLI entry shape for this theme.
    #[must_use]
    pub fn summary(&self, selected: bool) -> ThemeSummary {
        let manifest = &self.inspected().manifest;
        ThemeSummary {
            id: manifest.id.clone(),
            game: manifest.game.clone(),
            name: manifest.name.clone(),
            theme_api: manifest.theme_api,
            variants: manifest.variants.clone(),
            icon: manifest.icon.clone(),
            source: self.source().clone(),
            selected,
        }
    }
}

/// The `{"themes":[...],"diagnostics":[...]}` answer of `modn theme list`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListOutput {
    themes: Vec<ThemeSummary>,
    diagnostics: Vec<ThemeProblem>,
}

impl ListOutput {
    /// Listed themes in catalog order.
    #[must_use]
    pub fn themes(&self) -> &[ThemeSummary] {
        &self.themes
    }

    /// Diagnostics for skipped themes.
    #[must_use]
    pub fn diagnostics(&self) -> &[ThemeProblem] {
        &self.diagnostics
    }
}

impl ThemeCatalog {
    /// Build the list answer, marking stored selections.
    ///
    /// With a game filter, diagnostics that name another game are left out.
    /// An unlabelled bucket problem is kept because it may concern any game.
    #[must_use]
    pub fn list_output(&self, game: Option<&str>, selections: &[Selection]) -> ListOutput {
        let themes = self
            .list(game)
            .into_iter()
            .map(|entry| {
                let selected = selections.iter().any(|selection| {
                    selection.game() == entry.game() && selection.theme_id() == Some(entry.id())
                });
                entry.summary(selected)
            })
            .collect();
        let diagnostics = match game {
            None => self.problems().to_vec(),
            Some(game) => {
                let prefix = format!("{game}/");
                self.problems()
                    .iter()
                    .filter(|problem| {
                        problem
                            .theme
                            .as_deref()
                            .is_none_or(|theme| theme.starts_with(&prefix))
                    })
                    .cloned()
                    .collect()
            }
        };
        ListOutput {
            themes,
            diagnostics,
        }
    }
}
