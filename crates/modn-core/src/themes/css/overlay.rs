use super::forbidden_var::contains_var;

const OFFSET_PROPERTIES: &[&str] = &["top", "right", "bottom", "left", "inset"];

const PLACE_PREFIXES: &[&str] = &["inset-", "margin"];

const STACKING_PROPERTY: &str = "z-index";

#[must_use]
pub(crate) fn position_keeps_place(value: &str) -> bool {
    let value = value.trim();
    value.eq_ignore_ascii_case("static") || value.eq_ignore_ascii_case("relative")
}

#[must_use]
pub(crate) fn stacks_over(property: &str, value: &str) -> bool {
    if property != STACKING_PROPERTY || contains_var(value) {
        return false;
    }
    let value = value.trim();
    !(value.eq_ignore_ascii_case("auto") || value.parse::<i32>().is_ok_and(|level| level <= 0))
}

#[must_use]
pub(crate) fn moves_out_of_place(property: &str, value: &str) -> bool {
    if !is_place_property(property) || contains_var(value) {
        return false;
    }
    value
        .split([' ', ',', '/'])
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .any(|token| token.contains('(') || token.starts_with('-'))
}

fn is_place_property(property: &str) -> bool {
    OFFSET_PROPERTIES.contains(&property)
        || PLACE_PREFIXES
            .iter()
            .any(|prefix| property.starts_with(prefix))
}
