//! Parsing and validation of `theme.json`.

use super::limits::{
    is_valid_asset_path, is_valid_id, MAX_THEME_MANIFEST_BYTES, MAX_THEME_NAME_CHARS,
    MAX_THEME_VARIANTS,
};
use super::problem::{Rule, ThemeProblem};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Name of the manifest file inside a theme directory.
pub(super) const MANIFEST_FILE: &str = "theme.json";

/// The validated contents of one `theme.json`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeManifest {
    /// Manifest format version; must be `1`.
    pub schema_version: u32,
    /// Theme id; it must equal the theme directory name.
    pub id: String,
    /// Game id; it must equal the parent directory name.
    pub game: String,
    /// Display name of 1 to 64 characters without control characters.
    pub name: String,
    /// Theme API version; must be `1`.
    pub theme_api: u32,
    /// Variant ids, at most 16 and without duplicates.
    pub variants: Vec<String>,
    /// Optional icon path under `assets/` ending in `.png` or `.webp`.
    pub icon: Option<String>,
}

impl ThemeManifest {
    /// Parse and validate one `theme.json` document.
    ///
    /// An unknown JSON field is [`Rule::UnknownManifestField`]. Every other
    /// parse or validation failure is [`Rule::ManifestInvalid`].
    pub fn parse(bytes: &[u8]) -> Result<Self, ThemeProblem> {
        if bytes.len() as u64 > MAX_THEME_MANIFEST_BYTES {
            return Err(invalid(format!(
                "theme.json exceeds the {MAX_THEME_MANIFEST_BYTES}-byte limit"
            )));
        }
        let manifest: Self = serde_json::from_slice(bytes).map_err(|error| decode_error(&error))?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<(), ThemeProblem> {
        if self.schema_version != 1 {
            return Err(invalid(format!(
                "schema_version must be 1, found {}",
                self.schema_version
            )));
        }
        if self.theme_api != 1 {
            return Err(invalid(format!(
                "theme_api must be 1, found {}",
                self.theme_api
            )));
        }
        check_id(&self.id, "theme id")?;
        check_id(&self.game, "game id")?;
        if self.variants.len() > MAX_THEME_VARIANTS {
            return Err(invalid(format!(
                "variants must contain at most {MAX_THEME_VARIANTS} entries"
            )));
        }
        let mut seen = BTreeSet::new();
        for variant in &self.variants {
            check_id(variant, "variant id")?;
            if !seen.insert(variant) {
                return Err(invalid(format!(
                    "variant \"{variant}\" is listed more than once"
                )));
            }
        }
        let name_chars = self.name.chars().count();
        if name_chars == 0 || name_chars > MAX_THEME_NAME_CHARS {
            return Err(invalid(format!(
                "name must contain 1 to {MAX_THEME_NAME_CHARS} characters"
            )));
        }
        if self.name.chars().any(char::is_control) {
            return Err(invalid("name must not contain control characters"));
        }
        if let Some(icon) = &self.icon {
            if !is_valid_asset_path(icon) || !(icon.ends_with(".png") || icon.ends_with(".webp")) {
                return Err(invalid(format!(
                    "icon \"{icon}\" must be a path under assets/ ending in .png or .webp"
                )));
            }
        }
        Ok(())
    }
}

fn check_id(value: &str, kind: &str) -> Result<(), ThemeProblem> {
    if !is_valid_id(value) {
        return Err(invalid(format!("{kind} \"{value}\" is not a valid id")));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> ThemeProblem {
    ThemeProblem::new(Rule::ManifestInvalid, message).with_path(MANIFEST_FILE)
}

fn decode_error(error: &serde_json::Error) -> ThemeProblem {
    let text = error.to_string();
    if let Some(field) = unknown_field(&text) {
        return ThemeProblem::new(
            Rule::UnknownManifestField,
            format!("theme.json field \"{field}\" is not allowed"),
        )
        .with_path(MANIFEST_FILE);
    }
    ThemeProblem::new(
        Rule::ManifestInvalid,
        format!("theme.json is not valid: {text}"),
    )
    .with_path(MANIFEST_FILE)
}

/// Extract the field name from serde's `unknown field \`name\`...` message.
fn unknown_field(message: &str) -> Option<&str> {
    const MARKER: &str = "unknown field `";
    let start = message.find(MARKER)? + MARKER.len();
    let rest = &message[start..];
    let end = rest.find('`')?;
    Some(&rest[..end])
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;
