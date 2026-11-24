//! The `forbidden-overlay` rule: values that move or stack a themed element
//! over another element.
//!
//! The install review and the approval prompt are ordinary elements. A theme
//! that positions, offsets, stacks or grows an element outside its own box can
//! paint a fake prompt over the real one. Cascade layers do not stop that,
//! because the covering element is a different element.

use super::forbidden_var::contains_var;

/// Properties whose value offsets an element from its flow position.
const OFFSET_PROPERTIES: &[&str] = &["top", "right", "bottom", "left", "inset"];

/// Property prefixes whose value can move an element out of its place.
const PLACE_PREFIXES: &[&str] = &["inset-", "margin"];

/// The property that stacks an element over another element.
const STACKING_PROPERTY: &str = "z-index";

/// Return whether a `position` value keeps the element in its normal flow.
#[must_use]
pub(crate) fn position_keeps_place(value: &str) -> bool {
    let value = value.trim();
    value.eq_ignore_ascii_case("static") || value.eq_ignore_ascii_case("relative")
}

/// Return whether a declaration stacks an element above another element.
///
/// `property` must already be lowercase. A literal `auto`, `0` or a negative
/// integer stays allowed. A `var()` is the `forbidden-var` rule, so this
/// function does not report it again.
#[must_use]
pub(crate) fn stacks_over(property: &str, value: &str) -> bool {
    if property != STACKING_PROPERTY || contains_var(value) {
        return false;
    }
    let value = value.trim();
    !(value.eq_ignore_ascii_case("auto") || value.parse::<i32>().is_ok_and(|level| level <= 0))
}

/// Return whether a declaration moves an element out of its place.
///
/// `property` must already be lowercase. A negative length, or a value the
/// checker cannot resolve, could slide an element over a protected region, so
/// both are refused. A `var()` is the `forbidden-var` rule, so this function
/// does not report it again.
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

/// Return whether the property moves the element within or outside its box.
fn is_place_property(property: &str) -> bool {
    OFFSET_PROPERTIES.contains(&property)
        || PLACE_PREFIXES
            .iter()
            .any(|prefix| property.starts_with(prefix))
}
