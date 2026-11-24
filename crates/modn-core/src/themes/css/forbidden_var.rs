//! The `forbidden-var` value rule.
//!
//! A variable can turn an allowed declaration into a banned placement, size
//! or visibility value. Properties that protected content depends on must
//! therefore use literal values.

/// Properties that must not read a variable with `var()`.
const FORBIDDEN_VAR_PROPERTIES: &[&str] = &[
    "position",
    "display",
    "visibility",
    "opacity",
    "overflow",
    "overflow-x",
    "overflow-y",
    "z-index",
    "top",
    "right",
    "bottom",
    "left",
    "inset",
    "inset-block",
    "inset-inline",
    "width",
    "height",
    "min-width",
    "min-height",
    "max-width",
    "max-height",
    "font-size",
    "line-height",
];

/// Return whether `property` must use a literal value, ignoring case.
#[must_use]
pub(crate) fn forbids_var(property: &str) -> bool {
    FORBIDDEN_VAR_PROPERTIES
        .iter()
        .any(|candidate| property.eq_ignore_ascii_case(candidate))
}

/// Return whether `value` contains a `var()` function, ignoring case.
#[must_use]
pub(crate) fn contains_var(value: &str) -> bool {
    value.to_ascii_lowercase().contains("var(")
}
