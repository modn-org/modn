//! The theme API v1 property allowlist from `docs/themes.md`.

/// Properties that the allowlist names exactly.
const EXACT_PROPERTIES: &[&str] = &[
    "align-content",
    "align-items",
    "align-self",
    "aspect-ratio",
    "background",
    "background-clip",
    "background-color",
    "background-image",
    "background-position",
    "background-repeat",
    "background-size",
    "border",
    "border-radius",
    "bottom",
    "box-shadow",
    "box-sizing",
    "color",
    "column-gap",
    "display",
    "flex",
    "font",
    "gap",
    "grid",
    "height",
    "inset",
    "justify-content",
    "justify-items",
    "justify-self",
    "left",
    "letter-spacing",
    "line-height",
    "margin",
    "max-height",
    "max-width",
    "min-height",
    "min-width",
    "opacity",
    "order",
    "outline",
    "overflow",
    "padding",
    "place-content",
    "place-items",
    "place-self",
    "position",
    "right",
    "row-gap",
    "text-align",
    "text-decoration",
    "text-indent",
    "text-overflow",
    "text-shadow",
    "text-transform",
    "top",
    "transition",
    "visibility",
    "white-space",
    "width",
    "word-spacing",
    "z-index",
];

/// Prefix families that the allowlist names with a trailing `*`.
const PREFIX_PROPERTIES: &[&str] = &[
    "align-",
    "border-",
    "flex-",
    "font-",
    "grid-",
    "inset-",
    "justify-",
    "margin-",
    "outline-",
    "overflow",
    "padding-",
    "place-",
    "text-decoration-",
    "transition-",
];

/// Return whether `property` is on the theme API v1 allowlist.
///
/// `property` must already be lowercase. The allowlist matches exact names and
/// the documented prefix families; it never matches `background-*` by prefix.
#[must_use]
pub(crate) fn is_allowed_property(property: &str) -> bool {
    EXACT_PROPERTIES.contains(&property)
        || PREFIX_PROPERTIES
            .iter()
            .any(|prefix| property.starts_with(prefix))
}

#[cfg(test)]
#[path = "props_tests.rs"]
mod tests;
