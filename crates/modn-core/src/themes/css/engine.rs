//! The theme API v1 rule engine over parser-independent stylesheet facts.

use super::facts::{DeclFacts, FontFaceFacts, MediaFacts, RuleFacts, SheetFacts, StyleFacts};
use super::forbidden_var::{contains_var, forbids_var};
use super::media::valid_media_query;
use super::overlay::{moves_out_of_place, position_keeps_place, stacks_over};
use super::props::is_allowed_property;
use super::selector::{check_selector, SelectorInfo};
use super::token_range;
use super::value::{
    colour_is_opaque, find_url_targets, forbidden_image_function, is_remote_url,
    normalise_asset_target,
};
use crate::themes::problem::{Rule, ThemeProblem};
use std::collections::BTreeSet;

/// Properties accepted inside `@font-face`.
const FONT_FACE_PROPERTIES: &[&str] = &[
    "src",
    "font-family",
    "font-weight",
    "font-style",
    "font-display",
];

/// Properties accepted on a protected hook.
const PROTECTED_PROPERTIES: &[&str] = &[
    "color",
    "background-color",
    "border-color",
    "outline-color",
    "font-weight",
    "font-style",
];

/// Properties accepted on a hook that holds or surrounds a protected region.
const CONTEXT_PROPERTIES: &[&str] = &[
    "color",
    "background-color",
    "border-color",
    "outline-color",
    "font-family",
    "font-weight",
    "font-style",
];

/// Protected properties whose value must be an opaque colour.
const PROTECTED_COLOUR_PROPERTIES: &[&str] =
    &["color", "background-color", "border-color", "outline-color"];

/// The result of a stylesheet that passed every rule.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleOutcome {
    /// Every allowed asset URL, sorted and without duplicates.
    pub asset_urls: Vec<String>,
}

impl RuleOutcome {
    /// Create an outcome from canonical asset URLs.
    #[must_use]
    pub fn new(asset_urls: Vec<String>) -> Self {
        Self { asset_urls }
    }
}

/// Check one stylesheet and collect every problem it has.
///
/// `file` is the stylesheet path relative to the theme directory. Every
/// returned problem carries that path and the source line from `sheet`.
///
/// # Errors
///
/// Returns every rule violation in source order. It never stops at the first
/// one.
pub fn check_sheet(file: &str, sheet: &SheetFacts) -> Result<RuleOutcome, Vec<ThemeProblem>> {
    let mut checker = Checker {
        file,
        problems: Vec::new(),
        assets: BTreeSet::new(),
    };
    for rule in &sheet.rules {
        checker.check_rule(rule);
    }
    if checker.problems.is_empty() {
        Ok(RuleOutcome::new(checker.assets.into_iter().collect()))
    } else {
        Err(checker.problems)
    }
}

/// Accumulated state for one stylesheet check.
struct Checker<'a> {
    file: &'a str,
    problems: Vec<ThemeProblem>,
    assets: BTreeSet<String>,
}

impl Checker<'_> {
    /// Check one top-level rule.
    fn check_rule(&mut self, rule: &RuleFacts) {
        match rule {
            RuleFacts::Style(style) => self.check_style(style),
            RuleFacts::Media(media) => self.check_media(media),
            RuleFacts::FontFace(face) => self.check_font_face(face),
            RuleFacts::Unsupported(unsupported) => {
                let (rule, message) = if unsupported.at_rule == "import" {
                    (
                        Rule::ForbiddenImport,
                        "@import is outside the theme API".to_string(),
                    )
                } else {
                    (
                        Rule::ForbiddenAtRule,
                        format!("@{} is outside the theme API", unsupported.at_rule),
                    )
                };
                self.push(unsupported.line, rule, message);
            }
        }
    }

    /// Check a style rule and all of its declarations.
    fn check_style(&mut self, style: &StyleFacts) {
        let mut info = SelectorInfo {
            root_only: false,
            protected: false,
            context: false,
        };
        for selector in &style.selectors {
            match check_selector(selector, &style.declarations) {
                Ok(found) => {
                    info.root_only |= found.root_only;
                    info.protected |= found.protected;
                    info.context |= found.context;
                }
                Err(violation) => self.push(style.line, violation.rule, violation.message),
            }
        }
        for declaration in &style.declarations {
            self.check_declaration(declaration, info);
        }
    }

    /// Check one declaration of a style rule.
    fn check_declaration(&mut self, declaration: &DeclFacts, scope: SelectorInfo) {
        if declaration.important {
            self.push(
                declaration.line,
                Rule::ForbiddenImportant,
                "!important is not allowed in a theme",
            );
        }
        if declaration.property == "content" {
            self.push(
                declaration.line,
                Rule::ForbiddenContent,
                "the content property is not allowed in a theme",
            );
        }
        if declaration.property == "position"
            && !contains_var(&declaration.value)
            && !position_keeps_place(&declaration.value)
        {
            self.push(
                declaration.line,
                Rule::ForbiddenPositionFixed,
                format!(
                    "position: {} is not allowed in a theme; only static and relative keep an element in place",
                    declaration.value.trim()
                ),
            );
        }
        if stacks_over(&declaration.property, &declaration.value) {
            self.push(
                declaration.line,
                Rule::ForbiddenOverlay,
                format!(
                    "z-index: {} is not allowed in a theme; a theme element cannot stack over another element",
                    declaration.value.trim()
                ),
            );
        }
        if moves_out_of_place(&declaration.property, &declaration.value) {
            self.push(
                declaration.line,
                Rule::ForbiddenOverlay,
                format!(
                    "{}: {} moves an element out of its place; use a zero or positive literal length",
                    declaration.property,
                    declaration.value.trim()
                ),
            );
        }
        if forbids_var(&declaration.property) && contains_var(&declaration.value) {
            self.push(
                declaration.line,
                Rule::ForbiddenVar,
                "var() is not allowed in this property",
            );
        }
        if declaration.property.starts_with("--") {
            if !declaration.property.starts_with("--modn-") {
                self.push(
                    declaration.line,
                    Rule::ForbiddenCustomProperty,
                    format!(
                        "custom property \"{}\" must start with --modn-",
                        declaration.property
                    ),
                );
            }
            if let Some(range) = token_range::accepted_range(&declaration.property) {
                if !token_range::accepts(&declaration.property, &declaration.value) {
                    self.push(
                        declaration.line,
                        Rule::TokenOutOfRange,
                        format!("\"{}\" must be {range}", declaration.property),
                    );
                }
            }
        } else if declaration.property == "content" {
            // The dedicated forbidden-content rule already names it.
        } else if !is_allowed_property(&declaration.property) {
            self.push(
                declaration.line,
                Rule::UnknownProperty,
                format!(
                    "property \"{}\" is not on the theme API allowlist",
                    declaration.property
                ),
            );
        }
        self.check_urls(declaration.line, &declaration.value, None);
        if scope.protected {
            self.check_protected(declaration);
        }
        if scope.context && !context_allowed(declaration) && globally_allowed(declaration) {
            self.push(
                declaration.line,
                Rule::ProtectedRegion,
                format!(
                    "property \"{}\" is not allowed on a hook that holds a protected region",
                    declaration.property
                ),
            );
        }
        if protected_regions_read(declaration, scope) && !colour_is_opaque(&declaration.value) {
            self.push(
                declaration.line,
                Rule::ProtectedColorNotOpaque,
                format!(
                    "\"{}\" must be an opaque colour because protected regions read it",
                    declaration.property
                ),
            );
        }
    }

    /// Check one `@media` rule and its nested rules.
    fn check_media(&mut self, media: &MediaFacts) {
        if !valid_media_query(&media.query) {
            self.push(
                media.line,
                Rule::ForbiddenAtRule,
                format!("@media query \"{}\" is outside the theme API", media.query),
            );
        }
        for rule in &media.rules {
            match rule {
                RuleFacts::Style(style) => self.check_style(style),
                other => self.push(
                    line_of(other),
                    Rule::ForbiddenAtRule,
                    "@media may contain style rules only",
                ),
            }
        }
    }

    /// Check one `@font-face` rule.
    fn check_font_face(&mut self, face: &FontFaceFacts) {
        for declaration in &face.declarations {
            if !FONT_FACE_PROPERTIES.contains(&declaration.property.as_str()) {
                self.push(
                    declaration.line,
                    Rule::UnknownProperty,
                    format!(
                        "property \"{}\" is not allowed in @font-face",
                        declaration.property
                    ),
                );
            }
            let extension = (declaration.property == "src").then_some("woff2");
            self.check_urls(declaration.line, &declaration.value, extension);
        }
    }

    /// Check every `url()` and image-loading function in one value.
    fn check_urls(&mut self, line: u32, value: &str, required_extension: Option<&str>) {
        for target in find_url_targets(value) {
            if is_remote_url(target) {
                self.push(
                    line,
                    Rule::RemoteUrl,
                    format!("url({target}) points outside the theme"),
                );
                continue;
            }
            let Some(normalised) = normalise_asset_target(target) else {
                self.push(
                    line,
                    Rule::UrlOutsideAssets,
                    format!("url({target}) is not a theme asset"),
                );
                continue;
            };
            if required_extension.is_some_and(|extension| {
                !normalised
                    .to_ascii_lowercase()
                    .ends_with(&format!(".{extension}"))
            }) {
                self.push(
                    line,
                    Rule::UrlOutsideAssets,
                    format!("url({target}) does not end in .woff2"),
                );
                continue;
            }
            self.assets.insert(normalised);
        }
        if let Some(function) = forbidden_image_function(value) {
            self.push(
                line,
                Rule::UrlOutsideAssets,
                format!(
                    "{}() loads an image outside a theme asset",
                    function.trim_end_matches('(')
                ),
            );
        }
    }

    /// Check a declaration of a rule that names a protected hook.
    fn check_protected(&mut self, declaration: &DeclFacts) {
        if !PROTECTED_PROPERTIES.contains(&declaration.property.as_str()) {
            if globally_allowed(declaration) {
                self.push(
                    declaration.line,
                    Rule::ProtectedRegion,
                    format!(
                        "property \"{}\" is not allowed on a protected hook",
                        declaration.property
                    ),
                );
            }
            return;
        }
        if PROTECTED_COLOUR_PROPERTIES.contains(&declaration.property.as_str())
            && !colour_is_opaque(&declaration.value)
        {
            self.push(
                declaration.line,
                Rule::ProtectedColorNotOpaque,
                format!(
                    "value \"{}\" on a protected hook must be an opaque colour",
                    declaration.value
                ),
            );
        }
    }

    /// Record one problem with the stylesheet path and line.
    fn push(&mut self, line: u32, rule: Rule, message: impl Into<String>) {
        self.problems.push(
            ThemeProblem::new(rule, message)
                .with_path(self.file)
                .with_line(line),
        );
    }
}

/// Return whether a declaration feeds a colour that protected regions read.
///
/// Protected regions read the `--modn-color-*` tokens from `:root` and inherit
/// `color`, so a translucent value there reaches protected text or surfaces.
fn protected_regions_read(declaration: &DeclFacts, scope: SelectorInfo) -> bool {
    declaration.property.starts_with("--modn-color-")
        || (scope.root_only && declaration.property == "color")
}

/// Return whether a declaration is allowed on a context hook.
///
/// A context hook holds or surrounds a protected region, so it may change only
/// colours and fonts. Custom properties stay allowed for token definitions.
fn context_allowed(declaration: &DeclFacts) -> bool {
    declaration.property.starts_with("--modn-")
        || CONTEXT_PROPERTIES.contains(&declaration.property.as_str())
}

/// Return whether a declaration is allowed outside protected regions.
fn globally_allowed(declaration: &DeclFacts) -> bool {
    if declaration.property.starts_with("--") {
        declaration.property.starts_with("--modn-")
    } else {
        is_allowed_property(&declaration.property)
    }
}

/// Return the source line of any rule.
fn line_of(rule: &RuleFacts) -> u32 {
    match rule {
        RuleFacts::Style(style) => style.line,
        RuleFacts::Media(media) => media.line,
        RuleFacts::FontFace(face) => face.line,
        RuleFacts::Unsupported(unsupported) => unsupported.line,
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "engine_media_tests.rs"]
mod media_tests;

#[cfg(test)]
#[path = "engine_protected_tests.rs"]
mod protected_tests;

#[cfg(test)]
#[path = "engine_context_tests.rs"]
mod context_tests;

#[cfg(test)]
#[path = "engine_token_tests.rs"]
mod token_tests;

#[cfg(test)]
#[path = "engine_var_tests.rs"]
mod var_tests;

#[cfg(test)]
#[path = "engine_overlay_tests.rs"]
mod overlay_tests;
