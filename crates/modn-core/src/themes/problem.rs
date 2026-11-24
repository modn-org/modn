//! Stable rule ids and diagnostic records for theme validation.

use serde::ser::{Serialize, Serializer};
use std::fmt;

/// One stable diagnostic id from `docs/themes.md`.
///
/// The kebab-case identifiers returned by [`Rule::id`] are a public CLI
/// contract. Never rename a variant or its identifier.
#[non_exhaustive]
#[expect(
    clippy::enum_variant_names,
    reason = "Variant names mirror the documented rule ids, including the Rule suffix"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Rule {
    /// An `@import` rule.
    ForbiddenImport,
    /// An at-rule other than `@media` and `@font-face`.
    ForbiddenAtRule,
    /// A `url()` with a scheme, `//`, `data:` or a leading `/`.
    RemoteUrl,
    /// A `url()` outside `assets/` or with a rejected extension.
    UrlOutsideAssets,
    /// The `content` property.
    ForbiddenContent,
    /// `position: fixed`.
    ForbiddenPositionFixed,
    /// A `var()` in a property that must use a literal value.
    ForbiddenVar,
    /// The `!important` flag.
    ForbiddenImportant,
    /// A property outside the allowlist.
    UnknownProperty,
    /// A custom property not named `--modn-*`.
    ForbiddenCustomProperty,
    /// A size or opacity token outside its allowed value range.
    TokenOutOfRange,
    /// A selector outside the selector grammar.
    ForbiddenSelector,
    /// A `.modn-*` class or `data-*` attribute outside the stable tables.
    UnknownHook,
    /// A non-allowed property on a protected hook.
    ProtectedRegion,
    /// A translucent colour in a protected region.
    ProtectedColorNotOpaque,
    /// A CSS syntax error.
    CssSyntax,
    /// `theme.json` is missing, unreadable or malformed.
    ManifestInvalid,
    /// `theme.json` holds a field outside the documented table.
    UnknownManifestField,
    /// An id is invalid, or a directory name differs from the manifest.
    IdMismatch,
    /// `theme.css` is absent.
    MissingCss,
    /// A listed variant has no `theme.<variant>.css`.
    MissingVariantFile,
    /// A top-level entry is not allowed.
    UnexpectedFile,
    /// An asset extension or magic signature does not match.
    AssetType,
    /// One asset exceeds its size limit.
    AssetTooLarge,
    /// `assets/` holds more files than the limit.
    TooManyAssets,
    /// A stylesheet or the theme as a whole exceeds its size limit.
    ThemeTooLarge,
    /// A later duplicate `(game, id)` pair was skipped.
    DuplicateThemeId,
    /// A registered theme no longer matches its digest.
    DigestMismatch,
    /// The selected theme no longer exists.
    ThemeMissing,
}

impl Rule {
    /// Return the kebab-case identifier written to CLI and JSON output.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::ForbiddenImport => "forbidden-import",
            Self::ForbiddenAtRule => "forbidden-at-rule",
            Self::RemoteUrl => "remote-url",
            Self::UrlOutsideAssets => "url-outside-assets",
            Self::ForbiddenContent => "forbidden-content",
            Self::ForbiddenPositionFixed => "forbidden-position-fixed",
            Self::ForbiddenVar => "forbidden-var",
            Self::ForbiddenImportant => "forbidden-important",
            Self::UnknownProperty => "unknown-property",
            Self::ForbiddenCustomProperty => "forbidden-custom-property",
            Self::TokenOutOfRange => "token-out-of-range",
            Self::ForbiddenSelector => "forbidden-selector",
            Self::UnknownHook => "unknown-hook",
            Self::ProtectedRegion => "protected-region",
            Self::ProtectedColorNotOpaque => "protected-color-not-opaque",
            Self::CssSyntax => "css-syntax",
            Self::ManifestInvalid => "manifest-invalid",
            Self::UnknownManifestField => "unknown-manifest-field",
            Self::IdMismatch => "id-mismatch",
            Self::MissingCss => "missing-css",
            Self::MissingVariantFile => "missing-variant-file",
            Self::UnexpectedFile => "unexpected-file",
            Self::AssetType => "asset-type",
            Self::AssetTooLarge => "asset-too-large",
            Self::TooManyAssets => "too-many-assets",
            Self::ThemeTooLarge => "theme-too-large",
            Self::DuplicateThemeId => "duplicate-theme-id",
            Self::DigestMismatch => "digest-mismatch",
            Self::ThemeMissing => "theme-missing",
        }
    }
}

impl Serialize for Rule {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.id())
    }
}

/// One diagnostic about a theme.
///
/// Serializes to the CLI diagnostic object with exactly the keys `theme`,
/// `path`, `line`, `rule` and `message`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ThemeProblem {
    /// `<game>/<id>` once the manifest parsed, else `None`.
    pub theme: Option<String>,
    /// File path relative to the theme directory with `/` separators, else `None`.
    pub path: Option<String>,
    /// One-based line number when the validator knows it, else `None`.
    pub line: Option<u32>,
    /// Stable diagnostic id.
    pub rule: Rule,
    /// Human-readable explanation.
    pub message: String,
}

impl ThemeProblem {
    /// Create a problem for `rule` with an explanation and no location yet.
    #[must_use]
    pub fn new(rule: Rule, message: impl Into<String>) -> Self {
        Self {
            theme: None,
            path: None,
            line: None,
            rule,
            message: message.into(),
        }
    }

    /// Return this problem with its relative file path set.
    #[must_use]
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Return this problem with its one-based line number set.
    #[must_use]
    pub fn with_line(mut self, line: u32) -> Self {
        self.line = Some(line);
        self
    }

    /// Return this problem with its `<game>/<id>` theme label set.
    #[must_use]
    pub fn with_theme(mut self, theme: impl Into<String>) -> Self {
        self.theme = Some(theme.into());
        self
    }
}

impl fmt::Display for ThemeProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.rule.id(), self.message)?;
        if let Some(path) = &self.path {
            write!(f, " ({path}")?;
            if let Some(line) = self.line {
                write!(f, ":{line}")?;
            }
            write!(f, ")")?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "problem_tests.rs"]
mod tests;
