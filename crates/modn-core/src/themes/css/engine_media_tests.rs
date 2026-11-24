use super::check_sheet;
use crate::themes::css::facts::{
    DeclFacts, FontFaceFacts, MediaFacts, RuleFacts, SheetFacts, StyleFacts, UnsupportedFacts,
};
use crate::themes::Rule;

fn decl(property: &str, value: &str) -> DeclFacts {
    DeclFacts::new(property, value, false, 1)
}

fn style(selector: &str, declarations: Vec<DeclFacts>) -> RuleFacts {
    RuleFacts::Style(StyleFacts::new(vec![selector.to_string()], declarations, 1))
}

fn good_style() -> RuleFacts {
    style(".modn-card", vec![decl("display", "block")])
}

fn media(query: &str, rules: Vec<RuleFacts>) -> RuleFacts {
    RuleFacts::Media(MediaFacts::new(query, rules, 1))
}

fn check(rules: Vec<RuleFacts>) -> Result<super::RuleOutcome, Vec<crate::themes::ThemeProblem>> {
    check_sheet("theme.css", &SheetFacts::new(rules))
}

#[test]
fn accepted_media_queries_pass() {
    for query in [
        "(max-width: 900px)",
        "(min-width: 600px)",
        "(min-width: 600px) and (max-width: 900px)",
        "screen and (max-width: 900px)",
        "only screen and (min-width: 40em)",
        "not screen and (max-height: 500px)",
        "screen",
        "all",
        "screen, all",
        "(orientation: portrait)",
        "(orientation: landscape)",
        "(prefers-color-scheme: dark)",
        "(prefers-color-scheme: light)",
        "(prefers-reduced-motion: reduce)",
        "(prefers-reduced-motion: no-preference)",
        "(width >= 600px)",
        "(600px <= width)",
        "(height < 900px)",
        "(max-width: 60rem)",
        "(max-width: 0)",
        "SCREEN AND (MAX-WIDTH: 900PX)",
        "( min-width : 600px )",
    ] {
        let facts = vec![media(query, vec![good_style()])];
        assert!(check(facts).is_ok(), "query {query:?} should be accepted");
    }
}

#[test]
fn rejected_media_queries_fail_with_forbidden_at_rule() {
    for query in [
        "",
        "print",
        "screen and print",
        "(color)",
        "(hover: hover)",
        "(min-resolution: 2dppx)",
        "(width: calc(100px + 1px))",
        "(max-width: 900px",
        "(max-width: 900px) or (min-width: 600px)",
        "screen and (max-width: 900px) and",
        "(unknown-feature: 1)",
        "screen and (max-width: 900px) garbage",
        "and (max-width: 900px)",
    ] {
        let facts = vec![media(query, vec![good_style()])];
        let problems = check(facts).expect_err("query should be rejected");
        let found: Vec<Rule> = problems.iter().map(|problem| problem.rule).collect();
        assert_eq!(
            found,
            vec![Rule::ForbiddenAtRule],
            "query {query:?} should be forbidden-at-rule"
        );
        assert!(
            problems[0].message.contains(query),
            "message should name the query: {}",
            problems[0].message
        );
    }
}

#[test]
fn media_may_contain_style_rules_only() {
    for nested in [
        RuleFacts::FontFace(FontFaceFacts::new(
            vec![decl("src", "url(assets/a.woff2)")],
            2,
        )),
        RuleFacts::Unsupported(UnsupportedFacts::new("import", 2)),
        RuleFacts::Media(MediaFacts::new("screen", vec![good_style()], 2)),
    ] {
        let facts = vec![media("screen", vec![nested])];
        let problems = check(facts).expect_err("nested rule should be rejected");
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].rule, Rule::ForbiddenAtRule);
        assert_eq!(problems[0].line, Some(2));
    }
}

#[test]
fn media_style_rules_are_checked_in_full() {
    let facts = vec![media(
        "screen",
        vec![style(".modn-card", vec![decl("transform", "none")])],
    )];
    let problems = check(facts).expect_err("nested declaration should be rejected");
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].rule, Rule::UnknownProperty);
}

#[test]
fn an_invalid_query_still_reports_nested_problems() {
    let facts = vec![media(
        "print",
        vec![style(".modn-card", vec![decl("transform", "none")])],
    )];
    let rules: Vec<Rule> = check(facts)
        .expect_err("both problems should be reported")
        .into_iter()
        .map(|problem| problem.rule)
        .collect();
    assert_eq!(rules, [Rule::ForbiddenAtRule, Rule::UnknownProperty]);
}
