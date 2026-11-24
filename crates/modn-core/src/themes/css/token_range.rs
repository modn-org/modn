//! Value ranges for the size and opacity tokens that protected regions read.
//!
//! A theme may still tune these tokens, but only inside a range that keeps
//! protected text readable and interactive. Variables, keywords and values
//! the parser cannot read are refused.

/// One class of restricted token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenKind {
    FontSize,
    FontLine,
    Space,
    Opacity,
}

/// Return the accepted range of one restricted custom property.
#[must_use]
pub(crate) fn accepted_range(property: &str) -> Option<&'static str> {
    match kind(property) {
        Some(TokenKind::FontSize) => {
            Some("a length of at least 10px, or at least 0.625rem or 0.625em")
        }
        Some(TokenKind::FontLine) => {
            Some("a unitless number of at least 1, or a length of at least 10px")
        }
        Some(TokenKind::Space) => Some("a length from 0 to 64px, or from 0 to 4rem"),
        Some(TokenKind::Opacity) => Some("a number from 0.4 to 1"),
        None => None,
    }
}

/// Return whether `value` is a literal inside the range for `property`.
///
/// Properties outside every restricted group accept any value here.
#[must_use]
pub(crate) fn accepts(property: &str, value: &str) -> bool {
    let Some(kind) = kind(property) else {
        return true;
    };
    let Some(dimension) = parse_dimension(value) else {
        return false;
    };
    match (kind, dimension.number, dimension.unit) {
        (TokenKind::FontSize, number, Some("px")) => number >= 10.0,
        (TokenKind::FontSize, number, Some("rem" | "em")) => number >= 0.625,
        (TokenKind::FontLine, number, None) => number >= 1.0,
        (TokenKind::FontLine, number, Some("px")) => number >= 10.0,
        (TokenKind::Space, number, Some("px")) => (0.0..=64.0).contains(&number),
        (TokenKind::Space, number, Some("rem")) => (0.0..=4.0).contains(&number),
        (TokenKind::Space, number, None) => number == 0.0,
        (TokenKind::Opacity, number, None) => (0.4..=1.0).contains(&number),
        _ => false,
    }
}

/// Return the restricted group that `property` belongs to.
fn kind(property: &str) -> Option<TokenKind> {
    if prefix_ignore_case(property, "--modn-font-size-") {
        Some(TokenKind::FontSize)
    } else if prefix_ignore_case(property, "--modn-font-line-") {
        Some(TokenKind::FontLine)
    } else if prefix_ignore_case(property, "--modn-space-") {
        Some(TokenKind::Space)
    } else if property.eq_ignore_ascii_case("--modn-opacity-disabled") {
        Some(TokenKind::Opacity)
    } else {
        None
    }
}

/// Return whether `property` starts with `prefix`, ignoring ASCII case.
fn prefix_ignore_case(property: &str, prefix: &str) -> bool {
    property
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

/// One CSS number and its optional lowercase unit.
struct Dimension {
    number: f64,
    unit: Option<&'static str>,
}

/// Parse `value` as one number with an optional `px`, `rem` or `em` unit.
///
/// Whitespace, signs, decimals and leading zeros are accepted. Every other
/// unit, a function, a keyword or an empty value returns `None`.
fn parse_dimension(value: &str) -> Option<Dimension> {
    let value = value.trim();
    let split = value
        .find(|character: char| !(character.is_ascii_digit() || matches!(character, '+' | '-' | '.')))
        .unwrap_or(value.len());
    let (number, unit) = value.split_at(split);
    let number: f64 = number.parse().ok()?;
    let unit = match unit.trim().to_ascii_lowercase().as_str() {
        "" => None,
        "px" => Some("px"),
        "rem" => Some("rem"),
        "em" => Some("em"),
        _ => return None,
    };
    Some(Dimension { number, unit })
}

#[cfg(test)]
#[path = "token_range_tests.rs"]
mod tests;
