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
    "width",
    "height",
    "min-width",
    "min-height",
    "max-width",
    "max-height",
    "margin",
    "font-size",
    "line-height",
];

const FORBIDDEN_VAR_PREFIXES: &[&str] = &["inset-", "margin-"];

#[must_use]
pub(crate) fn forbids_var(property: &str) -> bool {
    let property = property.to_ascii_lowercase();
    FORBIDDEN_VAR_PROPERTIES
        .iter()
        .any(|candidate| property == *candidate)
        || FORBIDDEN_VAR_PREFIXES
            .iter()
            .any(|prefix| property.starts_with(prefix))
}

#[must_use]
pub(crate) fn contains_var(value: &str) -> bool {
    value.to_ascii_lowercase().contains("var(")
}
