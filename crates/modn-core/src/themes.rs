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

mod css;
mod digest;
mod inspect;
mod limits;
mod manifest;
mod problem;

pub use css::{CheckedCss, CssChecker, CssContext};
pub use digest::theme_digest;
pub use inspect::{inspect_theme_dir, AssetInfo, AssetKind, InspectedTheme};
pub use limits::{
    is_valid_asset_path, is_valid_id, MAX_THEME_ASSETS, MAX_THEME_ASSETS_BYTES,
    MAX_THEME_ASSET_BYTES, MAX_THEME_CSS_BYTES, MAX_THEME_MANIFEST_BYTES, MAX_THEME_VARIANTS,
    MAX_THEME_VARIANT_CSS_BYTES,
};
pub use manifest::ThemeManifest;
pub use problem::{Rule, ThemeProblem};
