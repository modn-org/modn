use super::check_selector;
use crate::themes::css::facts::DeclFacts;
use crate::themes::Rule;

/// Expected result for one selector table row.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Expect {
    Accepted,
    Rejected(Rule),
}

/// One selector table row.
struct Case {
    selector: &'static str,
    property: &'static str,
    expect: Expect,
}

fn declarations(property: &str) -> Vec<DeclFacts> {
    vec![DeclFacts::new(property, "#ffffff", false, 1)]
}

fn cases() -> Vec<Case> {
    use Expect::{Accepted, Rejected};
    let forbidden = Rejected(Rule::ForbiddenSelector);
    let unknown = Rejected(Rule::UnknownHook);
    vec![
        Case {
            selector: ".modn-card",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: ".modn-app .modn-card",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: ".modn-app > .modn-card",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: ".modn-card + .modn-card",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: ".modn-card ~ .modn-card",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: ".modn-card:hover",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: ".modn-card:focus-visible",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: ".modn-card:active",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: ".modn-button:disabled",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: ".modn-card:first-child",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: ".modn-card:last-child",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: ".modn-card:not(.modn-muted)",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: ".modn-card:not([data-state=\"busy\"])",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: "[data-theme=\"dark\"]",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: "[data-theme='dark']",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: "[data-theme=dark]",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: "[data-theme=\"dark\"] .modn-card",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: ".modn-card[data-state=\"busy\"]",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: "[data-modn-protected=\"approval\"]",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: ".modn-review__row .modn-muted",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: ".modn-card .modn-button--danger",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: ".modn-toasts .modn-toast",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: ":root",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: "html",
            property: "font-family",
            expect: Accepted,
        },
        Case {
            selector: "  .modn-card  ",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: ".modn-card:not(.modn-card):not(.modn-muted)",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: ":root .modn-card",
            property: "color",
            expect: Accepted,
        },
        Case {
            selector: "div .modn-card",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: "*",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: ".modn-card *",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: ".modn-card:has(.modn-muted)",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: ".modn-card:is(.modn-muted)",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: ".modn-card:where(.modn-muted)",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: ".modn-card::before",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: ".modn-card::after",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: ".modn-card:first-of-type",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: ".modn-card[data-state]",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: ".modn-card[data-state^=\"busy\"]",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: ".modn-card[data-state~=\"busy\"]",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: ".foo",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: ".modn-card.foo",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: "#modn-card",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: ".modn-card &",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: ".modn-card\\:hover",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: "svg|a",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: "html body",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: ":root",
            property: "display",
            expect: forbidden,
        },
        Case {
            selector: "html",
            property: "background",
            expect: forbidden,
        },
        Case {
            selector: ".modn-card:root",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: "",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: "   ",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: ".modn-card, .modn-muted",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: ":not(.modn-card)",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: ".modn-card:not(.foo)",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: ".modn-card:not()",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: ".modn-card:not(.modn-missing)",
            property: "color",
            expect: unknown,
        },
        Case {
            selector: ".modn-missing",
            property: "color",
            expect: unknown,
        },
        Case {
            selector: "[data-missing=\"x\"]",
            property: "color",
            expect: unknown,
        },
        Case {
            selector: ".modn-card[data-missing=\"x\"]",
            property: "color",
            expect: unknown,
        },
        Case {
            selector: ".modn-",
            property: "color",
            expect: unknown,
        },
        Case {
            selector: ".modn-card[data-theme=\"dark\" i]",
            property: "color",
            expect: forbidden,
        },
        Case {
            selector: ".modn-card[data-theme=\"dark\"] .",
            property: "color",
            expect: forbidden,
        },
    ]
}

#[test]
fn the_selector_table_holds_at_least_thirty_cases_with_both_outcomes() {
    let cases = cases();
    let accepted = cases
        .iter()
        .filter(|case| case.expect == Expect::Accepted)
        .count();
    assert!(cases.len() >= 30, "table should hold at least 30 cases");
    assert!(accepted >= 15, "half the table should be accepted");
    assert!(
        cases.len() - accepted >= 15,
        "half the table should be rejected"
    );
}

#[test]
fn each_selector_matches_its_expected_result() {
    for case in cases() {
        let result = check_selector(case.selector, &declarations(case.property));
        match (result, case.expect) {
            (Ok(_), Expect::Accepted) => {}
            (Err(violation), Expect::Rejected(rule)) => {
                assert_eq!(
                    violation.rule, rule,
                    "selector {:?} expected {rule:?}, got {:?}",
                    case.selector, violation.rule
                );
                assert!(
                    violation.message.contains(case.selector.trim())
                        || violation.message.contains("hook"),
                    "message should name the selector or hook: {}",
                    violation.message
                );
            }
            (Ok(_), Expect::Rejected(rule)) => {
                panic!(
                    "selector {:?} should be rejected with {rule:?}",
                    case.selector
                );
            }
            (Err(violation), Expect::Accepted) => {
                panic!(
                    "selector {:?} should be accepted, got {:?}",
                    case.selector, violation.rule
                );
            }
        }
    }
}

#[test]
fn accepted_selectors_report_protected_and_root_use() {
    let plain = check_selector(".modn-card", &declarations("color")).unwrap();
    assert!(!plain.protected);
    assert!(!plain.root_only);
    let protected = check_selector(".modn-review__status", &declarations("color")).unwrap();
    assert!(protected.protected);
    let attribute = check_selector(
        "[data-modn-protected=\"install-review\"]",
        &declarations("color"),
    )
    .unwrap();
    assert!(attribute.protected);
    let root = check_selector(":root", &declarations("color")).unwrap();
    assert!(root.root_only);
    let html = check_selector("html", &declarations("font-weight")).unwrap();
    assert!(html.root_only);
}

#[test]
fn root_rules_accept_only_root_properties() {
    for property in [
        "--modn-color-text",
        "color",
        "font",
        "font-size",
        "font-family",
    ] {
        assert!(
            check_selector(":root", &declarations(property)).is_ok(),
            "{property} should be allowed for :root"
        );
    }
    for property in ["display", "width", "--accent", "background"] {
        assert!(
            check_selector("html", &declarations(property)).is_err(),
            "{property} should be rejected for html"
        );
    }
}

#[test]
fn unknown_hook_messages_name_the_hook() {
    let violation = check_selector(".modn-missing", &declarations("color")).unwrap_err();
    assert_eq!(violation.rule, Rule::UnknownHook);
    assert!(violation.message.contains("modn-missing"));
}

#[test]
fn hostile_selectors_never_panic() {
    let long_class = format!(".modn-card{}", "a".repeat(100_000));
    let long_descendant = ".modn-card ".repeat(20_000);
    let unicode = ".modn-card\u{00e9}";
    for selector in [
        "",
        " ",
        ".",
        "[",
        ":",
        ":not(",
        ":not(.modn-card",
        "[]",
        long_class.as_str(),
        long_descendant.as_str(),
        unicode,
        "\u{0000}",
    ] {
        let _ = check_selector(selector, &declarations("color"));
    }
}
