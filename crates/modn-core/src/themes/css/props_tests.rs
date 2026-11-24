use super::is_allowed_property;

/// Every concrete property named by the allowlist table in `docs/themes.md`.
const ALLOWED: &[&str] = &[
    "display",
    "visibility",
    "position",
    "top",
    "right",
    "bottom",
    "left",
    "inset",
    "inset-inline",
    "inset-inline-start",
    "inset-block",
    "z-index",
    "flex",
    "flex-grow",
    "flex-shrink",
    "flex-basis",
    "flex-direction",
    "flex-wrap",
    "grid",
    "grid-template-columns",
    "grid-template-rows",
    "grid-column",
    "grid-row",
    "grid-area",
    "gap",
    "row-gap",
    "column-gap",
    "order",
    "align-items",
    "align-content",
    "align-self",
    "justify-items",
    "justify-content",
    "justify-self",
    "place-items",
    "place-content",
    "place-self",
    "overflow",
    "overflow-x",
    "overflow-y",
    "width",
    "height",
    "min-width",
    "min-height",
    "max-width",
    "max-height",
    "margin",
    "margin-top",
    "margin-inline",
    "padding",
    "padding-top",
    "padding-inline-start",
    "box-sizing",
    "aspect-ratio",
    "font",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "line-height",
    "letter-spacing",
    "word-spacing",
    "text-align",
    "text-transform",
    "text-decoration",
    "text-decoration-line",
    "text-decoration-color",
    "text-overflow",
    "text-indent",
    "white-space",
    "text-shadow",
    "color",
    "opacity",
    "background",
    "background-color",
    "background-image",
    "background-position",
    "background-size",
    "background-repeat",
    "background-clip",
    "border",
    "border-color",
    "border-top",
    "border-style",
    "border-width",
    "border-radius",
    "border-top-left-radius",
    "border-bottom-right-radius",
    "outline",
    "outline-color",
    "outline-offset",
    "outline-style",
    "outline-width",
    "box-shadow",
    "transition",
    "transition-duration",
    "transition-property",
    "transition-timing-function",
    "transition-delay",
];

/// Properties that the document names as forbidden.
const FORBIDDEN: &[&str] = &[
    "float",
    "animation",
    "animation-name",
    "animation-duration",
    "transform",
    "transform-origin",
    "filter",
    "backdrop-filter",
    "pointer-events",
    "cursor",
    "content",
    "background-origin",
    "background-attachment",
    "translate",
    "user-select",
    "-webkit-transform",
];

#[test]
fn every_documented_property_is_allowed() {
    for property in ALLOWED {
        assert!(
            is_allowed_property(property),
            "{property} should be on the allowlist"
        );
    }
}

#[test]
fn every_named_forbidden_property_is_rejected() {
    for property in FORBIDDEN {
        assert!(
            !is_allowed_property(property),
            "{property} should not be on the allowlist"
        );
    }
}

#[test]
fn prefixes_require_their_separator() {
    for property in ["bordermate", "flexible", "marginal", "gridd", "paddington"] {
        assert!(
            !is_allowed_property(property),
            "{property} should not match a prefix family"
        );
    }
}

#[test]
fn custom_properties_are_not_part_of_the_property_allowlist() {
    for property in ["--modn-color-text", "--accent"] {
        assert!(!is_allowed_property(property));
    }
}
