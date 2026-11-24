//! Theme limits and identifier grammars from `docs/themes.md`.

/// Maximum size of `theme.css` in bytes.
pub const MAX_THEME_CSS_BYTES: u64 = 262_144;

/// Maximum size of one `theme.<variant>.css` file in bytes.
pub const MAX_THEME_VARIANT_CSS_BYTES: u64 = 131_072;

/// Maximum size of one file under `assets/` in bytes.
pub const MAX_THEME_ASSET_BYTES: u64 = 2_097_152;

/// Maximum size of all files under `assets/` together in bytes.
pub const MAX_THEME_ASSETS_BYTES: u64 = 8_388_608;

/// Maximum number of files under `assets/`.
pub const MAX_THEME_ASSETS: usize = 64;

/// Maximum number of entries in `variants`.
pub const MAX_THEME_VARIANTS: usize = 16;

/// Maximum size of `theme.json` in bytes.
pub const MAX_THEME_MANIFEST_BYTES: u64 = 16 * 1024;

/// Maximum number of characters in a theme, game or variant id.
pub(crate) const MAX_THEME_ID_CHARS: usize = 64;

/// Maximum number of characters in a theme display name.
pub(crate) const MAX_THEME_NAME_CHARS: usize = 64;

/// Report whether `value` is a valid theme, game or variant id.
///
/// A valid id is lowercase ASCII `[a-z0-9]+(-[a-z0-9]+)*` with at most 64
/// characters.
#[must_use]
pub fn is_valid_id(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_THEME_ID_CHARS {
        return false;
    }
    let mut after_hyphen = true;
    for byte in value.bytes() {
        match byte {
            b'a'..=b'z' | b'0'..=b'9' => after_hyphen = false,
            b'-' if !after_hyphen => after_hyphen = true,
            _ => return false,
        }
    }
    !after_hyphen
}

/// Report whether `value` is an allowed path under `assets/`.
///
/// A valid path starts with `assets/` and then has only lowercase ASCII
/// letters, digits, `.`, `_` and `-` in non-empty `/`-separated segments. It
/// contains no `..` and no segment starts with `.`.
#[must_use]
pub fn is_valid_asset_path(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("assets/") else {
        return false;
    };
    if rest.is_empty() || value.contains("..") || value.contains('\\') {
        return false;
    }
    rest.split('/').all(|segment| {
        !segment.is_empty()
            && !segment.starts_with('.')
            && segment
                .bytes()
                .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
    })
}

#[cfg(test)]
#[path = "limits_tests.rs"]
mod tests;
