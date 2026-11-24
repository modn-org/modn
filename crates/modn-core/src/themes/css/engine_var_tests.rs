//! Rule checks for `var()` in properties that must use literal values.

use super::super::forbidden_var::{contains_var, forbids_var};
use super::check_sheet;
use crate::themes::css::facts::{DeclFacts, MediaFacts, RuleFacts, SheetFacts, StyleFacts};
use crate::themes::Rule;

fn decl(property: &str, value: &str, line: u32) -> DeclFacts {
    DeclFacts::new(property, value, false, line)
}

fn style(selector: &str, declarations: Vec<DeclFacts>, line: u32) -> RuleFacts {
    RuleFacts::Style(StyleFacts::new(
        vec![selector.to_string()],
        declarations,
        line,
    ))
}

fn rejecting(rules: Vec<RuleFacts>) -> Vec<Rule> {
    check_sheet("theme.css", &SheetFacts::new(rules))
        .expect_err("stylesheet should be rejected")
        .into_iter()
        .map(|problem| problem.rule)
        .collect()
}

#[test]
fn var_in_a_guarded_property_is_refused() {
    for property in [
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
    ] {
        let facts = vec![style(
            ".modn-card",
            vec![decl(property, "var(--modn-shared)", 7)],
            1,
        )];
        assert_eq!(
            rejecting(facts),
            [Rule::ForbiddenVar],
            "{property}: var() should be forbidden-var"
        );
    }
}

#[test]
fn a_var_position_bypass_is_refused() {
    let facts = vec![style(
        ".modn-rail",
        vec![decl("position", "var(--modn-position, fixed)", 5)],
        1,
    )];
    assert_eq!(rejecting(facts), [Rule::ForbiddenVar]);
}

#[test]
fn var_is_refused_inside_media_queries() {
    let media = RuleFacts::Media(MediaFacts::new(
        "(max-width: 900px)",
        vec![style(
            ".modn-rail",
            vec![decl("position", "var(--modn-position, fixed)", 5)],
            4,
        )],
        3,
    ));
    let problems = check_sheet("theme.css", &SheetFacts::new(vec![media]))
        .expect_err("stylesheet should be rejected");
    assert_eq!(problems.len(), 1, "one problem: {problems:?}");
    assert_eq!(problems[0].rule, Rule::ForbiddenVar);
    assert_eq!(problems[0].line, Some(5));
}

#[test]
fn var_stays_allowed_in_other_properties_and_custom_values() {
    for (property, value) in [
        ("padding", "var(--modn-space-4)"),
        ("margin", "var(--modn-space-s)"),
        ("color", "var(--modn-color-ink)"),
        ("background-color", "var(--modn-color-surface)"),
        ("grid-template-columns", "var(--modn-columns)"),
        ("--modn-radius-m", "var(--modn-radius-s)"),
        ("--modn-font-body", "var(--modn-font-heading)"),
    ] {
        let facts = vec![style(".modn-card", vec![decl(property, value, 1)], 1)];
        check_sheet("theme.css", &SheetFacts::new(facts)).unwrap_or_else(|problems| {
            panic!("{property}: {value} should be accepted: {problems:?}")
        });
    }
}

#[test]
fn the_var_check_ignores_case() {
    assert!(forbids_var("POSITION"));
    assert!(forbids_var("Line-Height"));
    assert!(!forbids_var("padding"));
    assert!(contains_var("VAR(--modn-position, fixed)"));
    assert!(contains_var("color: Var(--modn-color-ink)"));
    assert!(!contains_var("nothing here"));
}

#[test]
fn a_literal_fixed_position_keeps_its_own_rule() {
    let facts = vec![style(".modn-rail", vec![decl("position", "fixed", 1)], 1)];
    assert_eq!(rejecting(facts), [Rule::ForbiddenPositionFixed]);
}
