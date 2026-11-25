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
fn review_examples_are_refused() {
    for (selector, property, value) in [
        ("[data-operation=\"remove\"]", "display", "none"),
        ("[data-testid=\"ask-decline\"]", "display", "none"),
        (".modn-recent__menu", "opacity", "0"),
        (".modn-app", "opacity", "0"),
        (".modn-main", "opacity", "0"),
        (".modn-view", "opacity", "0"),
        (".modn-details", "visibility", "hidden"),
        (".modn-library", "visibility", "hidden"),
        (".modn-recent__menu", "overflow", "hidden"),
        (".modn-recent__menu", "height", "1px"),
    ] {
        let facts = vec![style(selector, vec![decl(property, value)])];
        assert_eq!(
            check(facts),
            [Rule::ProtectedRegion],
            "{selector} {{ {property}: {value} }} should be refused"
        );
    }
}

#[test]
fn a_positioned_context_hook_reports_both_rules() {
    let facts = vec![style(
        ".modn-recent__entry",
        vec![decl("position", "absolute")],
    )];
    assert_eq!(
        check(facts),
        [Rule::ForbiddenPositionFixed, Rule::ProtectedRegion]
    );
}

#[test]
fn hiding_the_details_view_hides_the_whole_review() {
    let facts = vec![
        style(".modn-details", vec![decl("visibility", "hidden")]),
        style(".modn-details__header", vec![decl("visibility", "visible")]),
    ];
    assert_eq!(check(facts), [Rule::ProtectedRegion]);
}

#[test]
fn context_hooks_accept_colours_on_the_main_ancestor() {
    for (selector, property, value) in [
        (".modn-main", "color", "#eeeeee"),
        (".modn-main", "background-color", "#101010"),
        (".modn-main", "font-family", "\"Night City\", sans-serif"),
        (".modn-view", "color", "#eeeeee"),
        (".modn-details", "border-color", "#333333"),
        (".modn-library", "outline-color", "#333333"),
    ] {
        accepts(selector, vec![decl(property, value)]);
    }
}

#[test]
fn reachable_attributes_are_refused_with_any_value() {
    for selector in [
        "[data-operation=\"write\"]",
        "[data-testid=\"ask-allow\"]",
        "[data-testid=\"details-plan\"]",
        "[data-code=\"internal\"]",
    ] {
        let facts = vec![style(selector, vec![decl("display", "none")])];
        assert_eq!(
            check(facts),
            [Rule::ProtectedRegion],
            "{selector} should be refused"
        );
    }
}

#[test]
fn context_hooks_accept_colours_and_fonts() {
    for (selector, property, value) in [
        (".modn-app", "background-color", "#101010"),
        (".modn-app", "color", "#eee"),
        (".modn-recent__entry", "color", "#eee"),
        (".modn-recent__menu", "border-color", "#333"),
        (".modn-app", "font-family", "\"Night City\", sans-serif"),
        (".modn-app", "font-weight", "600"),
        (".modn-app", "--modn-color-surface", "#101010"),
    ] {
        accepts(selector, vec![decl(property, value)]);
    }
}

#[test]
fn reachable_attributes_accept_colours() {
    accepts("[data-operation=\"remove\"]", vec![decl("color", "#eee")]);
    accepts("[data-testid=\"ask-decline\"]", vec![decl("color", "#eee")]);
}

#[test]
fn hooks_outside_the_context_list_keep_layout() {
    accepts(".modn-card", vec![decl("display", "flex")]);
    accepts(".modn-app .modn-card", vec![decl("opacity", "0.5")]);
}

#[test]
fn context_hooks_stay_limited_with_pseudo_classes() {
    for selector in [".modn-app:hover", ".modn-recent__menu:focus-visible"] {
        let facts = vec![style(selector, vec![decl("opacity", "0")])];
        assert_eq!(
            check(facts),
            [Rule::ProtectedRegion],
            "{selector} should keep the context limit"
        );
    }
}

#[test]
fn multiple_selectors_union_the_context_flag() {
    let facts = vec![RuleFacts::Style(StyleFacts::new(
        vec![".modn-card".to_string(), ".modn-app".to_string()],
        vec![decl("display", "grid")],
        1,
    ))];
    assert_eq!(check(facts), [Rule::ProtectedRegion]);
}
