//! Validation of theme directories and theme API v1 stylesheets.
//!
//! A theme is low-trust bucket content: CSS, images and fonts only. It cannot
//! run code, load remote resources or change a destructive confirmation. This
//! module reads one theme directory, reports every structural problem with a
//! stable rule id and, when the directory is valid, returns its parsed
//! manifest, canonical stylesheets, assets and BLAKE3 digest.
//!
//! CSS parsing lives behind the [`CssChecker`] seam. A later rule engine
//! implements that trait; this module only calls it and checks the assets it
//! reports.

mod catalog;
mod css;
mod digest;
mod error;
mod hooks;
mod inspect;
mod limits;
mod manifest;
mod problem;
mod registry;
mod resolve;
mod scan;
mod selection;
mod source;
mod storage;
mod summary;
#[cfg(test)]
mod test_support;

pub use catalog::{CatalogEntry, ThemeCatalog};
pub use css::{
    check_sheet, CheckedCss, CssChecker, CssContext, DeclFacts, FontFaceFacts, LightningChecker,
    MediaFacts, RuleFacts, RuleOutcome, SheetFacts, StyleFacts, UnsupportedFacts,
};
pub use digest::theme_digest;
pub use error::ThemeError;
pub use hooks::{
    is_protected_class, is_stable_class, is_stable_data_attribute, PROTECTED_CLASS_PREFIXES,
    PROTECTED_REGION_ATTRIBUTE, STABLE_CLASSES, STABLE_CUSTOM_PROPERTIES, STABLE_DATA_ATTRIBUTES,
};
pub use inspect::{inspect_theme_dir, AssetInfo, AssetKind, InspectedTheme};
pub use limits::{
    is_valid_asset_path, is_valid_id, MAX_THEME_ASSETS, MAX_THEME_ASSETS_BYTES,
    MAX_THEME_ASSET_BYTES, MAX_THEME_CSS_BYTES, MAX_THEME_MANIFEST_BYTES, MAX_THEME_VARIANTS,
    MAX_THEME_VARIANT_CSS_BYTES,
};
pub use manifest::ThemeManifest;
pub use problem::{Rule, ThemeProblem};
pub use registry::{ListedThemes, RegisteredTheme, RemovedTheme, ThemeRegistry};
pub use resolve::{Fallback, FallbackReason, Resolved, ResolvedOrigin, ResolvedTheme};
pub use scan::{scan_bucket, BucketScan, BucketThemeRoot};
pub use selection::{Selection, SelectionStore};
pub use source::{ThemeSource, ThemeSourceKind};
pub use summary::{ListOutput, ThemeSummary};
