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

#[test]
fn protected_hooks_accept_the_six_allowed_properties() {
    for (property, value) in [
        ("color", "#ffffff"),
        ("background-color", "#000000"),
        ("border-color", "#111111"),
        ("outline-color", "#222222"),
        ("font-weight", "600"),
        ("font-style", "italic"),
    ] {
        let facts = vec![style(".modn-review", vec![decl(property, value)])];
        let outcome = check_sheet("theme.css", &SheetFacts::new(facts))
            .expect("protected property should be accepted");
        assert!(outcome.asset_urls.is_empty());
    }
}

#[test]
fn font_family_is_refused_on_a_protected_hook() {
    let facts = vec![style(
        ".modn-approval__question",
        vec![decl("font-family", "x")],
    )];
    assert_eq!(check(facts), [Rule::ProtectedRegion]);
    let facts = vec![style(
        ".modn-approval__question",
        vec![decl("font-family", "\"Night City\", sans-serif")],
    )];
    assert_eq!(check(facts), [Rule::ProtectedRegion]);
}

#[test]
fn font_family_stays_allowed_on_a_shared_hook() {
    let facts = vec![style(
        ".modn-card",
        vec![decl("font-family", "\"Night City\", sans-serif")],
    )];
    check_sheet("theme.css", &SheetFacts::new(facts))
        .expect("font-family outside a protected hook stays allowed");
}

#[test]
fn other_allowed_properties_are_protected_region() {
    for (property, value) in [
        ("display", "none"),
        ("opacity", "0.5"),
        ("margin", "0"),
        ("background-image", "url(assets/a.png)"),
        ("border", "0"),
        ("font-family", "\"Night City\""),
        ("--modn-color-primary", "#ffffff"),
    ] {
        let facts = vec![style(".modn-review", vec![decl(property, value)])];
        assert_eq!(
            check(facts),
            [Rule::ProtectedRegion],
            "{property} should be protected-region"
        );
    }
}

#[test]
fn properties_outside_the_global_allowlist_stay_unknown_property() {
    let facts = vec![style(".modn-review", vec![decl("transform", "none")])];
    assert_eq!(check(facts), [Rule::UnknownProperty]);
    let facts = vec![style(".modn-approval", vec![decl("float", "left")])];
    assert_eq!(check(facts), [Rule::UnknownProperty]);
    let facts = vec![style(".modn-review", vec![decl("content", "\"x\"")])];
    assert_eq!(check(facts), [Rule::ForbiddenContent]);
}

#[test]
fn any_compound_naming_a_protected_hook_protects_the_rule() {
    for selector in [
        ".modn-review .modn-muted",
        ".modn-review__row .modn-muted",
        ".modn-app .modn-approval__button",
        "[data-modn-protected=\"approval\"] .modn-card",
        ".modn-card[data-modn-protected=\"install-review\"]",
        ".modn-card:not(.modn-review)",
    ] {
        let facts = vec![style(selector, vec![decl("display", "none")])];
        assert_eq!(
            check(facts),
            [Rule::ProtectedRegion],
            "{selector} should protect the rule"
        );
    }
}

#[test]
fn opaque_protected_colours_are_accepted() {
    for value in [
        "#fff",
        "#ffffff",
        "#ffff",
        "#ffffffff",
        "rgb(0, 0, 0)",
        "rgba(0, 0, 0, 1)",
        "rgb(0 0 0 / 1)",
        "rgb(0 0 0 / 100%)",
        "hsl(0, 0%, 0%)",
        "hsla(0 0% 0% / 100%)",
        "white",
        "rebeccapurple",
    ] {
        let facts = vec![style(".modn-review", vec![decl("color", value)])];
        assert!(
            check_sheet("theme.css", &SheetFacts::new(facts)).is_ok(),
            "{value} should be accepted"
        );
    }
}

#[test]
fn translucent_protected_colours_are_rejected() {
    for (property, value) in [
        ("color", "#ffffff00"),
        ("color", "#fff0"),
        ("color", "rgb(0 0 0 / 0.5)"),
        ("color", "rgba(0, 0, 0, 0.5)"),
        ("background-color", "hsl(0 0% 0% / 50%)"),
        ("border-color", "transparent"),
        ("outline-color", "currentcolor"),
        ("background-color", "var(--modn-color-surface)"),
        ("color", "inherit"),
        ("color", "color-mix(in srgb, red, blue)"),
    ] {
        let facts = vec![style(".modn-review", vec![decl(property, value)])];
        assert_eq!(
            check(facts),
            [Rule::ProtectedColorNotOpaque],
            "{property}: {value} should be rejected"
        );
    }
}

#[test]
fn translucent_colours_outside_protected_regions_are_allowed() {
    let facts = vec![style(
        ".modn-card",
        vec![decl("color", "rgba(0, 0, 0, 0.5)")],
    )];
    assert!(check_sheet("theme.css", &SheetFacts::new(facts)).is_ok());
}

#[test]
fn important_on_a_protected_hook_reports_only_important() {
    let facts = vec![RuleFacts::Style(StyleFacts::new(
        vec![".modn-review".to_string()],
        vec![DeclFacts::new("color", "#ffffff", true, 1)],
        1,
    ))];
    assert_eq!(check(facts), [Rule::ForbiddenImportant]);
}
