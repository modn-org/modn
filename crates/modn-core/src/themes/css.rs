//! The seam between theme loading and the CSS rule engine.

use super::problem::ThemeProblem;

/// Extra context for one stylesheet check.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssContext {
    /// Variant the stylesheet belongs to, or `None` for `theme.css`.
    pub variant: Option<String>,
}

impl CssContext {
    /// Context for the base `theme.css` stylesheet.
    #[must_use]
    pub fn base() -> Self {
        Self { variant: None }
    }

    /// Context for one `theme.<variant>.css` stylesheet.
    #[must_use]
    pub fn for_variant(variant: impl Into<String>) -> Self {
        Self {
            variant: Some(variant.into()),
        }
    }
}

/// One canonical stylesheet and the assets it references.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedCss {
    /// Canonical CSS text that the application may apply.
    pub text: String,
    /// Every referenced asset path, each in the form `assets/<path>`.
    pub asset_urls: Vec<String>,
}

impl CheckedCss {
    /// Create a checked stylesheet from canonical text and its asset paths.
    #[must_use]
    pub fn new(text: impl Into<String>, asset_urls: Vec<String>) -> Self {
        Self {
            text: text.into(),
            asset_urls,
        }
    }
}

/// Checks and canonicalises one theme stylesheet.
///
/// An implementation must reject every construct outside theme API v1 as
/// described in `docs/themes.md`, and must return the canonical text the
/// application will apply. In canonical text every allowed `url()` is
/// `url(assets/<path>)`. A problem may leave `path` empty; the caller fills in
/// the stylesheet file name.
pub trait CssChecker {
    /// Check `css`, the contents of `file`, under `ctx`.
    ///
    /// `asset_urls` must list every asset the stylesheet references in the
    /// form `assets/<path>` so the loader can confirm that each file exists.
    fn check(
        &self,
        file: &str,
        css: &str,
        ctx: &CssContext,
    ) -> Result<CheckedCss, Vec<ThemeProblem>>;
}
