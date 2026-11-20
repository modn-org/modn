//! Rule checks for colour tokens and root colours that protected regions read.

use super::check_sheet;
use crate::themes::css::facts::{DeclFacts, MediaFacts, RuleFacts, SheetFacts, StyleFacts};
use crate::themes::{Rule, ThemeProblem};

fn decl_at(property: &str, value: &str, line: u32) -> DeclFacts {
    DeclFacts::new(property, value, false, line)
}

fn style_at(selector: &str, declarations: Vec<DeclFacts>, line: u32) -> RuleFacts {
    RuleFacts::Style(StyleFacts::new(
        vec![selector.to_string()],
        declarations,
        line,
    ))
}

fn rejecting(rules: Vec<RuleFacts>) -> Vec<ThemeProblem> {
    check_sheet("theme.css", &SheetFacts::new(rules)).expect_err("stylesheet should be rejected")
}

fn assert_translucent(property: &str, value: &str, line: u32) -> ThemeProblem {
    let facts = style_at(".modn-app", vec![decl_at(property, value, line)], 1);
    let problems = rejecting(vec![facts]);
    assert_eq!(
        problems.len(),
        1,
        "{property}: {value} should report one problem: {problems:?}"
    );
    assert_eq!(problems[0].rule, Rule::ProtectedColorNotOpaque);
    assert_eq!(problems[0].line, Some(line));
    assert!(
        problems[0].message.contains(property),
        "message names {property}: {}",
        problems[0].message
    );
    assert!(
        problems[0].message.contains("protected regions read it"),
        "message explains the rule: {}",
        problems[0].message
    );
    problems.into_iter().next().expect("one problem")
}

#[test]
fn translucent_root_colours_are_refused() {
    for (selector, property, value) in [
        (":root", "--modn-color-text", "rgba(0, 0, 0, 0)"),
        (":root", "color", "transparent"),
        ("html", "color", "rgba(0, 0, 0, 0.5)"),
    ] {
        let facts = style_at(selector, vec![decl_at(property, value, 7)], 1);
        let problems = rejecting(vec![facts]);
        assert_eq!(
            problems.len(),
            1,
            "{selector} {{{property}: {value}}} should report one problem: {problems:?}"
        );
        assert_eq!(problems[0].rule, Rule::ProtectedColorNotOpaque);
        assert_eq!(problems[0].line, Some(7));
        assert!(problems[0].message.contains(property));
        assert!(problems[0].message.contains("protected regions read it"));
    }
}

#[test]
fn translucent_colour_tokens_are_refused_on_any_selector() {
    for (property, value) in [
        ("--modn-color-text", "#0000"),
        ("--modn-color-primary", "hsla(0, 0%, 0%, 0.5)"),
        ("--modn-color-surface", "rgba(0, 0, 0, 0)"),
        ("--modn-color-border", "color-mix(in srgb, red, blue)"),
        ("--modn-color-text-muted", "var(--modn-color-surface)"),
    ] {
        assert_translucent(property, value, 7);
    }
}

#[test]
fn translucent_root_token_inside_media_is_refused() {
    let media = RuleFacts::Media(MediaFacts::new(
        "(max-width: 900px)",
        vec![style_at(
            ":root",
            vec![decl_at("--modn-color-surface", "transparent", 5)],
            4,
        )],
        3,
    ));
    let problems = rejecting(vec![media]);
    assert_eq!(problems.len(), 1, "one problem: {problems:?}");
    assert_eq!(problems[0].rule, Rule::ProtectedColorNotOpaque);
    assert_eq!(problems[0].line, Some(5));
}

#[test]
fn a_root_selector_beside_a_hook_still_checks_root_colour() {
    let facts = RuleFacts::Style(StyleFacts::new(
        vec![".modn-card".to_string(), ":root".to_string()],
        vec![decl_at("color", "transparent", 9)],
        8,
    ));
    let problems = rejecting(vec![facts]);
    assert_eq!(problems.len(), 1, "one problem: {problems:?}");
    assert_eq!(problems[0].rule, Rule::ProtectedColorNotOpaque);
    assert_eq!(problems[0].line, Some(9));
}

#[test]
fn opaque_root_colours_are_accepted() {
    for (selector, property, value) in [
        (":root", "--modn-color-text", "#ffffff"),
        (":root", "color", "#ffffff"),
        (":root", "--modn-color-primary", "rgb(0, 0, 0)"),
        ("html", "color", "rebeccapurple"),
    ] {
        let facts = vec![style_at(selector, vec![decl_at(property, value, 1)], 1)];
        check_sheet("theme.css", &SheetFacts::new(facts))
            .unwrap_or_else(|problems| panic!("{selector} {property}: {value}: {problems:?}"));
    }
}

#[test]
fn translucent_colours_on_shared_hooks_stay_valid() {
    for (selector, property, value) in [
        (".modn-card", "color", "rgba(0, 0, 0, 0.5)"),
        (".modn-card", "background-color", "rgba(0, 0, 0, 0.5)"),
        (":root", "--modn-space-m", "12px"),
        (".modn-app", "--modn-font-line-body-s", "1.5"),
    ] {
        let facts = vec![style_at(selector, vec![decl_at(property, value, 1)], 1)];
        check_sheet("theme.css", &SheetFacts::new(facts))
            .unwrap_or_else(|problems| panic!("{selector} {property}: {value}: {problems:?}"));
    }
}

/// Document a known gap: protected content reads spacing, radius, font-size,
/// font-line and opacity tokens, and the rule engine accepts any value for
/// them. A theme can set `:root{--modn-font-size-body-s:0}` and shrink or hide
/// protected text. Do not delete this test; it fails when the gap closes.
#[test]
#[ignore = "known gap: protected regions read non-colour tokens that accept any value"]
fn known_gap_root_tokens_can_shrink_protected_content() {
    for (property, value) in [
        ("--modn-font-size-body-s", "0"),
        ("--modn-font-line-body-s", "0"),
        ("--modn-space-4xl", "0"),
        ("--modn-opacity-disabled", "0"),
    ] {
        let facts = vec![style_at(":root", vec![decl_at(property, value, 1)], 1)];
        check_sheet("theme.css", &SheetFacts::new(facts))
            .unwrap_or_else(|problems| panic!("{property}: {value}: {problems:?}"));
    }
}
