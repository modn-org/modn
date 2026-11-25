#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenKind {
    FontSize,
    FontLine,
    Space,
    Opacity,
}

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

fn prefix_ignore_case(property: &str, prefix: &str) -> bool {
    property
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

struct Dimension {
    number: f64,
    unit: Option<&'static str>,
}

fn parse_dimension(value: &str) -> Option<Dimension> {
    let value = value.trim();
    for (index, _) in value
        .char_indices()
        .chain(std::iter::once((value.len(), '\0')))
    {
        let (number, unit) = value.split_at(index);
        let Ok(number) = number.parse::<f64>() else {
            continue;
        };
        if !number.is_finite() {
            continue;
        }
        let unit = match unit.trim().to_ascii_lowercase().as_str() {
            "" => None,
            "px" => Some("px"),
            "rem" => Some("rem"),
            "em" => Some("em"),
            _ => continue,
        };
        return Some(Dimension { number, unit });
    }
    None
}

#[cfg(test)]
#[path = "token_range_tests.rs"]
mod tests;
