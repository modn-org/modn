use super::engine::check_sheet;
use super::facts::{
    DeclFacts, FontFaceFacts, MediaFacts, RuleFacts, SheetFacts, StyleFacts, UnsupportedFacts,
};
use super::value::normalise_asset_target;
use super::{CheckedCss, CssChecker, CssContext};
use crate::themes::problem::{Rule, ThemeProblem};
use lightningcss::declaration::DeclarationBlock;
use lightningcss::properties::Property;
use lightningcss::rules::font_face::FontFaceProperty;
use lightningcss::rules::style::StyleRule;
use lightningcss::rules::{CssRule, CssRuleList, Location};
use lightningcss::stylesheet::{ParserFlags, ParserOptions, PrinterOptions, StyleSheet};
use lightningcss::traits::ToCss;

const MAX_MESSAGE_CHARS: usize = 200;

/// Checks and canonicalises theme CSS with the `lightningcss` parser.
///
/// Parsing keeps source locations, so every problem carries the one-based
/// line of the rule it concerns. A syntax error stops the check with one
/// `css-syntax` problem.
#[derive(Debug, Clone, Copy, Default)]
#[expect(
    clippy::exhaustive_structs,
    reason = "the checker holds no state, so callers construct it directly"
)]
pub struct LightningChecker;

impl CssChecker for LightningChecker {
    fn check(
        &self,
        file: &str,
        css: &str,
        _ctx: &CssContext,
    ) -> Result<CheckedCss, Vec<ThemeProblem>> {
        let sheet = match parse(file, css) {
            Ok(sheet) => sheet,
            Err(problem) => return Err(vec![problem]),
        };
        let facts = match convert(&sheet.rules) {
            Ok(rules) => SheetFacts::new(rules),
            Err(problem) => return Err(vec![problem.with_path(file)]),
        };
        let outcome = check_sheet(file, &facts)?;
        match sheet.to_css(PrinterOptions::default()) {
            Ok(result) => Ok(CheckedCss::new(
                canonicalise_urls(&result.code),
                outcome.asset_urls,
            )),
            Err(error) => Err(vec![syntax(1, error.kind.to_string())]),
        }
    }
}

fn parse<'i>(file: &str, css: &'i str) -> Result<StyleSheet<'i>, ThemeProblem> {
    let options = ParserOptions {
        filename: file.to_owned(),
        error_recovery: false,
        flags: ParserFlags::NESTING,
        ..ParserOptions::default()
    };
    StyleSheet::parse(css, options).map_err(|error| {
        let line = error.loc.map_or(1, |loc| loc.line + 1);
        syntax(line, cut(&error.kind.to_string(), MAX_MESSAGE_CHARS))
    })
}

fn convert(rules: &CssRuleList<'_>) -> Result<Vec<RuleFacts>, ThemeProblem> {
    rules.0.iter().map(convert_rule).collect()
}

fn convert_rule(rule: &CssRule<'_>) -> Result<RuleFacts, ThemeProblem> {
    match rule {
        CssRule::Style(style) => convert_style(style),
        CssRule::Media(media) => {
            let line = one_based(media.loc);
            let query = media
                .query
                .to_css_string(PrinterOptions::default())
                .map_err(|error| syntax(line, error.kind.to_string()))?;
            Ok(RuleFacts::Media(MediaFacts::new(
                query,
                convert(&media.rules)?,
                line,
            )))
        }
        CssRule::FontFace(face) => {
            let line = one_based(face.loc);
            Ok(RuleFacts::FontFace(FontFaceFacts::new(
                convert_font_face(&face.properties, line)?,
                line,
            )))
        }
        other => {
            let (name, line) = unsupported(other);
            Ok(RuleFacts::Unsupported(UnsupportedFacts::new(name, line)))
        }
    }
}

fn convert_style(rule: &StyleRule<'_>) -> Result<RuleFacts, ThemeProblem> {
    let line = one_based(rule.loc);
    if !rule.rules.0.is_empty() {
        return Ok(RuleFacts::Unsupported(UnsupportedFacts::new(
            "nesting", line,
        )));
    }
    let mut selectors = Vec::with_capacity(rule.selectors.0.len());
    for selector in &rule.selectors.0 {
        selectors.push(
            selector
                .to_css_string(PrinterOptions::default())
                .map_err(|error| syntax(line, error.kind.to_string()))?,
        );
    }
    Ok(RuleFacts::Style(StyleFacts::new(
        selectors,
        convert_declarations(&rule.declarations, line)?,
        line,
    )))
}

/// Convert a declaration block to facts.
///
/// `lightningcss` stores `!important` declarations apart from the others, so
/// the facts list normal declarations first and then the important ones.
fn convert_declarations(
    block: &DeclarationBlock<'_>,
    line: u32,
) -> Result<Vec<DeclFacts>, ThemeProblem> {
    let mut facts =
        Vec::with_capacity(block.declarations.len() + block.important_declarations.len());
    for property in &block.declarations {
        facts.push(convert_property(property, false, line)?);
    }
    for property in &block.important_declarations {
        facts.push(convert_property(property, true, line)?);
    }
    Ok(facts)
}

fn convert_property(
    property: &Property<'_>,
    important: bool,
    line: u32,
) -> Result<DeclFacts, ThemeProblem> {
    let value = property
        .value_to_css_string(PrinterOptions::default())
        .map_err(|error| syntax(line, error.kind.to_string()))?;
    Ok(DeclFacts::new(
        property.property_id().name().to_ascii_lowercase(),
        value,
        important,
        line,
    ))
}

fn convert_font_face(
    properties: &[FontFaceProperty<'_>],
    line: u32,
) -> Result<Vec<DeclFacts>, ThemeProblem> {
    let mut facts = Vec::with_capacity(properties.len());
    for property in properties {
        let text = property
            .to_css_string(PrinterOptions::default())
            .map_err(|error| syntax(line, error.kind.to_string()))?;
        let Some((name, value)) = text.split_once(':') else {
            return Err(syntax(
                line,
                format!("@font-face descriptor \"{text}\" has no name"),
            ));
        };
        facts.push(DeclFacts::new(
            name.trim().to_ascii_lowercase(),
            value.trim(),
            false,
            line,
        ));
    }
    Ok(facts)
}

fn unsupported(rule: &CssRule<'_>) -> (String, u32) {
    match rule {
        CssRule::Import(rule) => ("import".to_owned(), one_based(rule.loc)),
        CssRule::Keyframes(rule) => ("keyframes".to_owned(), one_based(rule.loc)),
        CssRule::Supports(rule) => ("supports".to_owned(), one_based(rule.loc)),
        CssRule::Page(rule) => ("page".to_owned(), one_based(rule.loc)),
        CssRule::CounterStyle(rule) => ("counter-style".to_owned(), one_based(rule.loc)),
        CssRule::Namespace(rule) => ("namespace".to_owned(), one_based(rule.loc)),
        CssRule::MozDocument(rule) => ("-moz-document".to_owned(), one_based(rule.loc)),
        CssRule::Nesting(rule) => ("nesting".to_owned(), one_based(rule.loc)),
        CssRule::NestedDeclarations(rule) => ("nesting".to_owned(), one_based(rule.loc)),
        CssRule::Viewport(rule) => ("viewport".to_owned(), one_based(rule.loc)),
        CssRule::CustomMedia(rule) => ("custom-media".to_owned(), one_based(rule.loc)),
        CssRule::LayerStatement(rule) => ("layer".to_owned(), one_based(rule.loc)),
        CssRule::LayerBlock(rule) => ("layer".to_owned(), one_based(rule.loc)),
        CssRule::Property(rule) => ("property".to_owned(), one_based(rule.loc)),
        CssRule::Container(rule) => ("container".to_owned(), one_based(rule.loc)),
        CssRule::FontPaletteValues(rule) => ("font-palette-values".to_owned(), one_based(rule.loc)),
        CssRule::FontFeatureValues(rule) => ("font-feature-values".to_owned(), one_based(rule.loc)),
        CssRule::Scope(rule) => ("scope".to_owned(), one_based(rule.loc)),
        CssRule::StartingStyle(rule) => ("starting-style".to_owned(), one_based(rule.loc)),
        CssRule::ViewTransition(rule) => ("view-transition".to_owned(), one_based(rule.loc)),
        CssRule::PositionTry(rule) => ("position-try".to_owned(), one_based(rule.loc)),
        CssRule::Unknown(rule) => (rule.name.to_ascii_lowercase(), one_based(rule.loc)),
        // The parser never produces these with error recovery off.
        CssRule::Style(_) | CssRule::Media(_) | CssRule::FontFace(_) => ("unknown".to_owned(), 1),
        CssRule::Custom(_) => ("custom".to_owned(), 1),
        CssRule::Ignored => ("ignored".to_owned(), 1),
    }
}

fn one_based(loc: Location) -> u32 {
    loc.line + 1
}

/// Rewrite every quoted asset `url()` to the canonical unquoted form.
///
/// `lightningcss` always quotes a URL when it does not minify. Only a quoted
/// target that normalises to a plain `assets/<path>` is rewritten, so no other
/// text changes.
fn canonicalise_urls(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < bytes.len() {
        if index + 4 <= bytes.len() && bytes[index..index + 4].eq_ignore_ascii_case(b"url(") {
            if let Some((target, after)) = quoted_target(text, index + 4) {
                if plain_asset_path(target) {
                    out.push_str("url(");
                    out.push_str(target);
                    out.push(')');
                    index = after;
                    continue;
                }
            }
        }
        let character = text[index..]
            .chars()
            .next()
            .expect("index is on a character boundary");
        out.push(character);
        index += character.len_utf8();
    }
    out
}

fn quoted_target(text: &str, start: usize) -> Option<(&str, usize)> {
    let bytes = text.as_bytes();
    let mut index = start;
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    let quote = *bytes.get(index)?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let content_start = index + 1;
    let mut end = content_start;
    while end < bytes.len() && bytes[end] != quote {
        end += 1;
    }
    let mut after = end + 1;
    while after < bytes.len() && bytes[after].is_ascii_whitespace() {
        after += 1;
    }
    if after >= bytes.len() || bytes[after] != b')' {
        return None;
    }
    Some((&text[content_start..end], after + 1))
}

fn plain_asset_path(target: &str) -> bool {
    target
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'-' | b'_'))
        && normalise_asset_target(target).is_some()
}

fn cut(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

fn syntax(line: u32, message: impl Into<String>) -> ThemeProblem {
    ThemeProblem::new(Rule::CssSyntax, message).with_line(line)
}

#[cfg(test)]
#[path = "lightning_tests.rs"]
mod tests;
