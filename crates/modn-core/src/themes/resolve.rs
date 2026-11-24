//! Resolution of the theme that a game uses.

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;

use super::catalog::{CatalogEntry, ThemeCatalog};
use super::problem::{Rule, ThemeProblem};
use super::selection::Selection;
use super::source::{ThemeSource, ThemeSourceKind};
use serde::Serialize;

/// Why a selected theme was not used.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FallbackReason {
    /// The theme or its variant failed validation.
    ThemeInvalid,
    /// No catalog entry and no problem names the theme.
    ThemeMissing,
    /// A registered theme no longer matches its digest.
    DigestMismatch,
}

impl FallbackReason {
    /// Return the kebab-case identifier written to JSON.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::ThemeInvalid => "theme-invalid",
            Self::ThemeMissing => "theme-missing",
            Self::DigestMismatch => "digest-mismatch",
        }
    }
}

/// One failed step of the resolve order.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Fallback {
    reason: FallbackReason,
    message: String,
}

impl Fallback {
    /// Build a fallback explanation.
    #[must_use]
    pub(crate) fn new(reason: FallbackReason, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: message.into(),
        }
    }

    /// Why the selected theme was skipped.
    #[must_use]
    pub fn reason(&self) -> FallbackReason {
        self.reason
    }

    /// Human-readable explanation.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Where the resolved theme came from in the resolve order.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ResolvedOrigin {
    /// The stored selection for the game.
    Selected,
    /// The lexicographically first official theme for the game.
    Official,
}

/// The theme that the desktop must apply.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedTheme {
    id: String,
    name: String,
    source: ThemeSource,
    variant: Option<String>,
    origin: ResolvedOrigin,
    css: String,
    variant_css: Option<String>,
    assets_dir: String,
}

impl ResolvedTheme {
    /// Theme id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Display name of the theme.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Where the theme came from.
    #[must_use]
    pub fn source(&self) -> &ThemeSource {
        &self.source
    }

    /// Selected variant id, or `None` for the base stylesheet.
    #[must_use]
    pub fn variant(&self) -> Option<&str> {
        self.variant.as_deref()
    }

    /// Which step of the resolve order chose this theme.
    #[must_use]
    pub fn origin(&self) -> ResolvedOrigin {
        self.origin
    }

    /// Canonical `theme.css` text.
    #[must_use]
    pub fn css(&self) -> &str {
        &self.css
    }

    /// Canonical variant stylesheet, or `None`.
    #[must_use]
    pub fn variant_css(&self) -> Option<&str> {
        self.variant_css.as_deref()
    }

    /// Absolute path of the theme directory.
    #[must_use]
    pub fn assets_dir(&self) -> &str {
        &self.assets_dir
    }
}

/// The `modn theme resolve` answer.
///
/// `theme: None` means the default theme.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Resolved {
    game: String,
    theme: Option<ResolvedTheme>,
    fallback: Option<Fallback>,
    diagnostics: Vec<ThemeProblem>,
}

impl Resolved {
    /// The game that was resolved.
    #[must_use]
    pub fn game(&self) -> &str {
        &self.game
    }

    /// The theme to apply, or `None` for the default theme.
    #[must_use]
    pub fn theme(&self) -> Option<&ResolvedTheme> {
        self.theme.as_ref()
    }

    /// Why the stored selection was skipped, when it was.
    #[must_use]
    pub fn fallback(&self) -> Option<&Fallback> {
        self.fallback.as_ref()
    }

    /// Problems that explain the fallback or the resolved game.
    #[must_use]
    pub fn diagnostics(&self) -> &[ThemeProblem] {
        &self.diagnostics
    }
}

impl ThemeCatalog {
    /// Resolve the theme for `game`.
    ///
    /// Order: the selected theme when it is valid, then the official theme
    /// with the smallest id, then the default theme. A selection for another
    /// game is ignored. An explicit none selection returns the default theme
    /// without the official theme. A failed theme selection sets `fallback`
    /// and continues with the official theme.
    #[must_use]
    pub fn resolve(&self, game: &str, selection: Option<&Selection>) -> Resolved {
        let selection = selection.filter(|selection| selection.game() == game);
        if selection.is_some_and(Selection::is_none) {
            return Resolved {
                game: game.to_owned(),
                theme: None,
                fallback: None,
                diagnostics: self.problems_for_game(game),
            };
        }
        let (fallback, diagnostics) = match selection {
            Some(selection) => match self.selected_theme(game, selection) {
                Ok(theme) => {
                    return Resolved {
                        game: game.to_owned(),
                        theme: Some(theme),
                        fallback: None,
                        diagnostics: self.problems_for_game(game),
                    };
                }
                Err((fallback, diagnostics)) => (Some(fallback), diagnostics),
            },
            None => (None, self.problems_for_game(game)),
        };
        Resolved {
            game: game.to_owned(),
            theme: self.official_theme(game),
            fallback,
            diagnostics,
        }
    }

    /// Resolve one stored selection, or explain why it cannot be used.
    fn selected_theme(
        &self,
        game: &str,
        selection: &Selection,
    ) -> Result<ResolvedTheme, (Fallback, Vec<ThemeProblem>)> {
        let id = selection.theme();
        let Some(entry) = self.get(game, id) else {
            return Err(self.missing_selection(game, id));
        };
        if let Some(variant) = selection.variant() {
            if !entry
                .inspected()
                .manifest
                .variants
                .iter()
                .any(|listed| listed == variant)
            {
                let fallback = Fallback::new(
                    FallbackReason::ThemeInvalid,
                    format!("theme \"{id}\" has no variant \"{variant}\""),
                );
                return Err((fallback, self.problems_for_theme(game, id)));
            }
        }
        Ok(resolved_theme(
            entry,
            selection.variant(),
            ResolvedOrigin::Selected,
        ))
    }

    /// Explain a selection that has no catalog entry.
    fn missing_selection(&self, game: &str, id: &str) -> (Fallback, Vec<ThemeProblem>) {
        let mut diagnostics = self.problems_for_theme(game, id);
        let reason = if diagnostics
            .iter()
            .any(|problem| problem.rule == Rule::DigestMismatch)
        {
            FallbackReason::DigestMismatch
        } else if diagnostics.is_empty() {
            diagnostics.push(
                ThemeProblem::new(
                    Rule::ThemeMissing,
                    format!("the selected theme \"{id}\" for game \"{game}\" is not available"),
                )
                .with_theme(format!("{game}/{id}")),
            );
            FallbackReason::ThemeMissing
        } else {
            FallbackReason::ThemeInvalid
        };
        let message = match reason {
            FallbackReason::DigestMismatch => diagnostics
                .iter()
                .find(|problem| problem.rule == Rule::DigestMismatch)
                .map(|problem| problem.message.clone()),
            FallbackReason::ThemeInvalid | FallbackReason::ThemeMissing => {
                diagnostics.first().map(|problem| problem.message.clone())
            }
        };
        let fallback = Fallback::new(
            reason,
            message.unwrap_or_else(|| format!("the selected theme \"{id}\" is not available")),
        );
        (fallback, diagnostics)
    }

    /// The official theme with the smallest id, with no variant.
    fn official_theme(&self, game: &str) -> Option<ResolvedTheme> {
        self.entries()
            .iter()
            .filter(|entry| {
                entry.game() == game && entry.source().kind() == ThemeSourceKind::Official
            })
            .min_by(|left, right| left.id().cmp(right.id()))
            .map(|entry| resolved_theme(entry, None, ResolvedOrigin::Official))
    }
}

/// Build the resolve record for one catalog entry.
fn resolved_theme(
    entry: &CatalogEntry,
    variant: Option<&str>,
    origin: ResolvedOrigin,
) -> ResolvedTheme {
    let inspected = entry.inspected();
    ResolvedTheme {
        id: inspected.manifest.id.clone(),
        name: inspected.manifest.name.clone(),
        source: entry.source().clone(),
        variant: variant.map(str::to_owned),
        origin,
        css: inspected.css.clone(),
        variant_css: variant.and_then(|variant| inspected.variant_css.get(variant).cloned()),
        assets_dir: inspected.root.to_string_lossy().into_owned(),
    }
}
