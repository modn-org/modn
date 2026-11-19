//! Parser-independent facts about one stylesheet.
//!
//! A CSS parser produces these facts, and [`super::engine::check_sheet`]
//! checks them against theme API v1. The facts hold serialised text only, so
//! the rule engine needs no CSS parser of its own.

/// Every rule of one stylesheet, in source order.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetFacts {
    /// Top-level rules in source order.
    pub rules: Vec<RuleFacts>,
}

impl SheetFacts {
    /// Create the facts of one stylesheet.
    #[must_use]
    pub fn new(rules: Vec<RuleFacts>) -> Self {
        Self { rules }
    }
}

/// One rule or at-rule of a stylesheet.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleFacts {
    /// A style rule.
    Style(StyleFacts),
    /// An `@media` rule.
    Media(MediaFacts),
    /// A `@font-face` rule.
    FontFace(FontFaceFacts),
    /// Any other at-rule.
    Unsupported(UnsupportedFacts),
}

/// A style rule with its selectors and declarations.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleFacts {
    /// One entry per comma-separated selector, as serialised text.
    pub selectors: Vec<String>,
    /// Declarations in source order.
    pub declarations: Vec<DeclFacts>,
    /// One-based source line of the rule.
    pub line: u32,
}

impl StyleFacts {
    /// Create a style rule.
    #[must_use]
    pub fn new(selectors: Vec<String>, declarations: Vec<DeclFacts>, line: u32) -> Self {
        Self {
            selectors,
            declarations,
            line,
        }
    }
}

/// An `@media` rule and its nested rules.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaFacts {
    /// The media query as serialised text, without the `@media` keyword.
    pub query: String,
    /// Nested rules in source order.
    pub rules: Vec<RuleFacts>,
    /// One-based source line of the rule.
    pub line: u32,
}

impl MediaFacts {
    /// Create an `@media` rule.
    #[must_use]
    pub fn new(query: impl Into<String>, rules: Vec<RuleFacts>, line: u32) -> Self {
        Self {
            query: query.into(),
            rules,
            line,
        }
    }
}

/// A `@font-face` rule.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFaceFacts {
    /// Declarations in source order.
    pub declarations: Vec<DeclFacts>,
    /// One-based source line of the rule.
    pub line: u32,
}

impl FontFaceFacts {
    /// Create a `@font-face` rule.
    #[must_use]
    pub fn new(declarations: Vec<DeclFacts>, line: u32) -> Self {
        Self { declarations, line }
    }
}

/// An at-rule outside the theme API.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedFacts {
    /// At-rule name without `@`, lowercase, for example `import`.
    pub at_rule: String,
    /// One-based source line of the rule.
    pub line: u32,
}

impl UnsupportedFacts {
    /// Create an unsupported at-rule.
    #[must_use]
    pub fn new(at_rule: impl Into<String>, line: u32) -> Self {
        Self {
            at_rule: at_rule.into(),
            line,
        }
    }
}

/// One declaration inside a style rule or `@font-face` rule.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclFacts {
    /// Property name, lowercase.
    pub property: String,
    /// Property value, as serialised text.
    pub value: String,
    /// Whether the declaration carries `!important`.
    pub important: bool,
    /// One-based source line of the declaration.
    pub line: u32,
}

impl DeclFacts {
    /// Create a declaration.
    #[must_use]
    pub fn new(
        property: impl Into<String>,
        value: impl Into<String>,
        important: bool,
        line: u32,
    ) -> Self {
        Self {
            property: property.into(),
            value: value.into(),
            important,
            line,
        }
    }
}
