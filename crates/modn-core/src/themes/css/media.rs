//! Validation of `@media` queries against the theme API grammar.
//!
//! `docs/themes.md` names the at-rule and its allowed contents but not the
//! media features. This module accepts the common feature names and plain
//! `and`, `not`, `only`, `screen` and `all` structure, plus the level 4 range
//! syntax with numbers and lengths.

/// Media feature names accepted by the theme API.
const MEDIA_FEATURES: &[&str] = &[
    "width",
    "height",
    "min-width",
    "max-width",
    "min-height",
    "max-height",
    "orientation",
    "prefers-color-scheme",
    "prefers-reduced-motion",
];

/// Return whether a media query stays inside the theme API grammar.
pub(crate) fn valid_media_query(query: &str) -> bool {
    let lower = query.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut index = 0;
    let mut last = LastToken::None;
    while index < bytes.len() {
        match bytes[index] {
            b' ' | b'\t' | b'\n' | b'\r' | 0x0c => index += 1,
            b',' => {
                if last != LastToken::Term {
                    return false;
                }
                last = LastToken::Connector;
                index += 1;
            }
            b'(' => {
                let Some(end) = matching_paren(&lower, index) else {
                    return false;
                };
                if !valid_media_feature(&lower[index + 1..end]) {
                    return false;
                }
                if last == LastToken::Term {
                    return false;
                }
                last = LastToken::Term;
                index = end + 1;
            }
            b'a'..=b'z' | b'-' => {
                let start = index;
                while index < bytes.len()
                    && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'-')
                {
                    index += 1;
                }
                let word = &lower[start..index];
                match word {
                    "and" => {
                        if last != LastToken::Term {
                            return false;
                        }
                        last = LastToken::Connector;
                    }
                    "not" | "only" => {
                        if last == LastToken::Term {
                            return false;
                        }
                        last = LastToken::Connector;
                    }
                    "screen" | "all" => {
                        if last == LastToken::Term {
                            return false;
                        }
                        last = LastToken::Term;
                    }
                    _ => return false,
                }
            }
            _ => return false,
        }
    }
    last == LastToken::Term
}

/// What the previous media query token was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LastToken {
    /// No token yet.
    None,
    /// A connector such as `and`, `not`, `only` or a comma.
    Connector,
    /// A term such as `screen`, `all` or a feature block.
    Term,
}

/// Return the index of the `)` that closes the `(` at `open`.
fn matching_paren(query: &str, open: usize) -> Option<usize> {
    let bytes = query.as_bytes();
    let mut index = open + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'(' => return None,
            b')' => return Some(index),
            _ => index += 1,
        }
    }
    None
}

/// Return whether one parenthesised media feature stays in the allowlist.
fn valid_media_feature(inner: &str) -> bool {
    let inner = inner.trim();
    if inner.is_empty()
        || inner.contains('(')
        || inner.contains(')')
        || inner.contains(['"', '\'', '\\'])
    {
        return false;
    }
    if let Some((name, value)) = inner.split_once(':') {
        if value.contains(':') {
            return false;
        }
        return MEDIA_FEATURES.contains(&name.trim()) && valid_media_value(value.trim());
    }
    let mut words = Vec::new();
    for token in inner.split_whitespace() {
        if matches!(token, ">=" | "<=" | ">" | "<" | "=") {
            continue;
        }
        for part in token.split(['>', '<', '=']) {
            if !part.is_empty() {
                words.push(part);
            }
        }
    }
    let mut saw_feature = false;
    let mut saw_value = false;
    for word in words {
        if MEDIA_FEATURES.contains(&word) {
            saw_feature = true;
        } else if is_number_or_length(word) {
            saw_value = true;
        } else {
            return false;
        }
    }
    saw_feature && saw_value
}

/// Return whether a media value is made of words, numbers and lengths.
fn valid_media_value(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'-' | b'_' | b'.' | b'%' | b'+' | b'/' | b' ')
        })
}

/// Return whether a token is a number or a number with a unit.
fn is_number_or_length(token: &str) -> bool {
    let mut saw_digit = false;
    for byte in token.bytes() {
        if byte.is_ascii_digit() {
            saw_digit = true;
        } else if !(byte.is_ascii_alphabetic() || matches!(byte, b'.' | b'%' | b'+' | b'-')) {
            return false;
        }
    }
    saw_digit
}
