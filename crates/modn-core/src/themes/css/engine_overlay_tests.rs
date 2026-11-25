use super::check_sheet;
use crate::themes::css::facts::{DeclFacts, RuleFacts, SheetFacts, StyleFacts};
use crate::themes::Rule;

fn decl(property: &str, value: &str) -> DeclFacts {
    DeclFacts::new(property, value, false, 1)
}

fn style(selector: &str, declarations: Vec<DeclFacts>) -> RuleFacts {
    RuleFacts::Style(StyleFacts::new(vec![selector.to_string()], declarations, 1))
}

fn check(rules: Vec<RuleFacts>) -> Vec<Rule> {
    check_sheet("theme.css", &SheetFacts::new(rules))
        .expect_err("stylesheet should be rejected")
        .into_iter()
        .map(|problem| problem.rule)
        .collect()
}

fn accepts(selector: &str, declarations: Vec<DeclFacts>) {
    let facts = vec![style(selector, declarations)];
    check_sheet("theme.css", &SheetFacts::new(facts))
        .unwrap_or_else(|problems| panic!("{selector} should be accepted: {problems:?}"));
}

#[test]
fn the_reported_toast_overlay_is_refused() {
    let facts = vec![style(
        ".modn-toasts",
        vec![
            decl("position", "absolute"),
            decl("top", "0"),
            decl("left", "0"),
            decl("width", "100vw"),
            decl("height", "100vh"),
            decl("z-index", "99"),
            decl("background-color", "#000"),
        ],
    )];
    assert_eq!(
        check(facts),
        [Rule::ForbiddenPositionFixed, Rule::ForbiddenOverlay]
    );
}

#[test]
fn the_reported_footer_overlay_is_refused() {
    let facts = vec![style(
        ".modn-details__footer",
        vec![
            decl("position", "absolute"),
            decl("top", "0"),
            decl("left", "0"),
            decl("width", "100vw"),
            decl("height", "100vh"),
            decl("z-index", "99"),
            decl("background-color", "#000"),
        ],
    )];
    assert_eq!(
        check(facts),
        [Rule::ForbiddenPositionFixed, Rule::ForbiddenOverlay]
    );
}

#[test]
fn every_position_besides_static_and_relative_is_refused() {
    for value in ["absolute", "fixed", "sticky"] {
        let facts = vec![style(".modn-card", vec![decl("position", value)])];
        assert_eq!(
            check(facts),
            [Rule::ForbiddenPositionFixed],
            "position: {value} should be refused"
        );
    }
}

#[test]
fn a_positive_z_index_is_refused_on_every_hook() {
    for (selector, value) in [
        (".modn-card", "1"),
        (".modn-toasts", "99"),
        (".modn-details__footer", "2147483647"),
        (".modn-card", "0.5"),
        (".modn-card", "calc(1)"),
    ] {
        let facts = vec![style(selector, vec![decl("z-index", value)])];
        assert_eq!(
            check(facts),
            [Rule::ForbiddenOverlay],
            "{selector} {{ z-index: {value} }} should be refused"
        );
    }
}

#[test]
fn a_var_z_index_keeps_the_var_rule() {
    let facts = vec![style(
        ".modn-card",
        vec![decl("z-index", "var(--modn-stack)")],
    )];
    assert_eq!(check(facts), [Rule::ForbiddenVar]);
}

#[test]
fn a_negative_offset_or_margin_is_refused() {
    for (property, value) in [
        ("top", "-1px"),
        ("right", "-0.5em"),
        ("bottom", "-10%"),
        ("left", "-2rem"),
        ("inset", "0 0 -10px 0"),
        ("inset-block", "-4px"),
        ("inset-inline-start", "-4px"),
        ("margin", "0 0 -8px 0"),
        ("margin-top", "-8px"),
        ("margin-inline-start", "-8px"),
    ] {
        let facts = vec![style(".modn-details__footer", vec![decl(property, value)])];
        assert_eq!(
            check(facts),
            [Rule::ForbiddenOverlay],
            "{property}: {value} should be refused"
        );
    }
}

#[test]
fn an_unresolved_offset_or_margin_is_refused() {
    for (property, value) in [
        ("top", "calc(0px - 4px)"),
        ("margin", "calc(-1 * 4px)"),
        ("margin-top", "max(0px, -4px)"),
        ("margin-top", "env(safe-area-inset-top)"),
    ] {
        let facts = vec![style(".modn-details__footer", vec![decl(property, value)])];
        assert_eq!(
            check(facts),
            [Rule::ForbiddenOverlay],
            "{property}: {value} should be refused"
        );
    }
}

#[test]
fn a_var_offset_or_margin_keeps_the_var_rule() {
    for (property, value) in [
        ("top", "var(--modn-offset)"),
        ("margin", "var(--modn-space-m)"),
        ("margin-top", "var(--modn-space-s)"),
    ] {
        let facts = vec![style(".modn-card", vec![decl(property, value)])];
        assert_eq!(
            check(facts),
            [Rule::ForbiddenVar],
            "{property}: {value} should be forbidden-var"
        );
    }
}

#[test]
fn a_zero_or_positive_position_stays_allowed() {
    for (property, value) in [
        ("position", "static"),
        ("position", "relative"),
        ("z-index", "auto"),
        ("z-index", "0"),
        ("z-index", "-1"),
        ("top", "0"),
        ("left", "12px"),
        ("inset", "0 0 10px 0"),
        ("inset-block-start", "4px"),
        ("margin", "0 auto"),
        ("margin", "12px auto 0 auto"),
        ("margin-top", "4px"),
        ("margin-inline-start", "8px"),
    ] {
        accepts(".modn-card", vec![decl(property, value)]);
    }
}
