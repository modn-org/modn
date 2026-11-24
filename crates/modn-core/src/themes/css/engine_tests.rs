use super::{check_sheet, RuleOutcome};
use crate::themes::css::facts::{
    DeclFacts, FontFaceFacts, MediaFacts, RuleFacts, SheetFacts, StyleFacts, UnsupportedFacts,
};
use crate::themes::{Rule, ThemeProblem};

fn decl(property: &str, value: &str) -> DeclFacts {
    DeclFacts::new(property, value, false, 1)
}

fn decl_at(property: &str, value: &str, important: bool, line: u32) -> DeclFacts {
    DeclFacts::new(property, value, important, line)
}

fn style(selector: &str, declarations: Vec<DeclFacts>) -> RuleFacts {
    RuleFacts::Style(StyleFacts::new(vec![selector.to_string()], declarations, 1))
}

fn good_style() -> RuleFacts {
    style(".modn-card", vec![decl("color", "#ffffff")])
}

fn check(rules: Vec<RuleFacts>) -> Result<RuleOutcome, Vec<ThemeProblem>> {
    check_sheet("theme.css", &SheetFacts::new(rules))
}

fn rejecting(rules: Vec<RuleFacts>) -> Vec<ThemeProblem> {
    check(rules).expect_err("stylesheet should be rejected")
}

fn rules_of(rules: Vec<RuleFacts>) -> Vec<Rule> {
    rejecting(rules)
        .into_iter()
        .map(|problem| problem.rule)
        .collect()
}

fn assert_single(rules: Vec<RuleFacts>, rule: Rule) {
    let problems = rejecting(rules);
    assert_eq!(problems.len(), 1, "expected one problem: {problems:?}");
    assert_eq!(problems[0].rule, rule);
}

#[test]
fn a_valid_stylesheet_is_accepted() {
    let outcome = check(vec![good_style()]).unwrap();
    assert!(outcome.asset_urls.is_empty());
}

#[test]
fn an_empty_stylesheet_is_accepted() {
    assert_eq!(check(Vec::new()).unwrap().asset_urls, Vec::<String>::new());
}

#[test]
fn forbidden_import_is_rejected() {
    assert_single(
        vec![RuleFacts::Unsupported(UnsupportedFacts::new("import", 3))],
        Rule::ForbiddenImport,
    );
    assert!(check(vec![good_style()]).is_ok());
}

#[test]
fn forbidden_at_rule_is_rejected() {
    assert_single(
        vec![RuleFacts::Unsupported(UnsupportedFacts::new("supports", 4))],
        Rule::ForbiddenAtRule,
    );
    let media = RuleFacts::Media(MediaFacts::new(
        "screen and (max-width: 900px)",
        vec![good_style()],
        1,
    ));
    assert!(check(vec![media]).is_ok());
}

#[test]
fn remote_url_is_rejected() {
    assert_single(
        vec![style(
            ".modn-card",
            vec![decl(
                "background-image",
                "url(\"https://example.com/a.png\")",
            )],
        )],
        Rule::RemoteUrl,
    );
    assert!(check(vec![style(
        ".modn-card",
        vec![decl("background-image", "url(assets/a.png)")],
    )])
    .is_ok());
}

#[test]
fn url_outside_assets_is_rejected() {
    assert_single(
        vec![style(
            ".modn-card",
            vec![decl("background-image", "url(\"../a.png\")")],
        )],
        Rule::UrlOutsideAssets,
    );
    assert_single(
        vec![style(
            ".modn-card",
            vec![decl("background-image", "image-set(assets/a.png)")],
        )],
        Rule::UrlOutsideAssets,
    );
    assert!(check(vec![style(
        ".modn-card",
        vec![decl("background-image", "url(assets/a.png)")],
    )])
    .is_ok());
}

#[test]
fn forbidden_content_is_rejected() {
    assert_single(
        vec![style(".modn-card", vec![decl("content", "\"new\"")])],
        Rule::ForbiddenContent,
    );
    assert!(check(vec![style(".modn-card", vec![decl("opacity", "0.9")])]).is_ok());
}

#[test]
fn forbidden_position_fixed_is_rejected() {
    for value in [
        "fixed",
        "absolute",
        "sticky",
        "static-ish",
        "var(--modn-position)",
    ] {
        let facts = vec![style(".modn-rail", vec![decl("position", value)])];
        if value.starts_with("var(") {
            assert_single(facts, Rule::ForbiddenVar);
        } else {
            assert_single(facts, Rule::ForbiddenPositionFixed);
        }
    }
    assert!(check(vec![style(
        ".modn-rail",
        vec![decl("position", "relative")]
    )])
    .is_ok());
    assert!(check(vec![style(".modn-rail", vec![decl("position", "static")])]).is_ok());
}

#[test]
fn forbidden_important_is_rejected() {
    assert_single(
        vec![style(
            ".modn-card",
            vec![decl_at("color", "#ffffff", true, 1)],
        )],
        Rule::ForbiddenImportant,
    );
    assert!(check(vec![style(".modn-card", vec![decl("color", "#ffffff")])]).is_ok());
}

#[test]
fn unknown_property_is_rejected() {
    assert_single(
        vec![style(".modn-card", vec![decl("transform", "scale(1.1)")])],
        Rule::UnknownProperty,
    );
    assert!(check(vec![style(".modn-card", vec![decl("opacity", "0.8")])]).is_ok());
}

#[test]
fn forbidden_custom_property_is_rejected() {
    assert_single(
        vec![style(".modn-app", vec![decl("--accent", "#ffffff")])],
        Rule::ForbiddenCustomProperty,
    );
    assert!(check(vec![style(
        ".modn-app",
        vec![decl("--modn-color-primary", "#ffffff")],
    )])
    .is_ok());
}

#[test]
fn forbidden_selector_is_rejected() {
    assert_single(
        vec![style("div .modn-card", vec![decl("color", "#ffffff")])],
        Rule::ForbiddenSelector,
    );
    assert!(check(vec![style(
        ".modn-app .modn-card",
        vec![decl("color", "#ffffff")],
    )])
    .is_ok());
}

#[test]
fn unknown_hook_is_rejected() {
    assert_single(
        vec![style(".modn-missing", vec![decl("color", "#ffffff")])],
        Rule::UnknownHook,
    );
    assert!(check(vec![style(".modn-card", vec![decl("color", "#ffffff")])]).is_ok());
}

#[test]
fn protected_region_is_rejected() {
    assert_single(
        vec![style(".modn-review", vec![decl("display", "none")])],
        Rule::ProtectedRegion,
    );
    assert!(check(vec![style(".modn-review", vec![decl("color", "#ffffff")])]).is_ok());
}

#[test]
fn protected_color_not_opaque_is_rejected() {
    assert_single(
        vec![style(
            ".modn-approval",
            vec![decl("background-color", "rgba(0, 0, 0, 0.5)")],
        )],
        Rule::ProtectedColorNotOpaque,
    );
    assert!(check(vec![style(
        ".modn-approval",
        vec![decl("background-color", "#000000")],
    )])
    .is_ok());
}

#[test]
fn documented_bad_examples_yield_exactly_their_rule() {
    let cases = vec![
        (
            Rule::ForbiddenImport,
            vec![RuleFacts::Unsupported(UnsupportedFacts::new("import", 1))],
        ),
        (
            Rule::ForbiddenAtRule,
            vec![RuleFacts::Unsupported(UnsupportedFacts::new("supports", 1))],
        ),
        (
            Rule::RemoteUrl,
            vec![style(
                ".modn-card",
                vec![decl(
                    "background-image",
                    "url(\"https://example.com/a.png\")",
                )],
            )],
        ),
        (
            Rule::UrlOutsideAssets,
            vec![style(
                ".modn-card",
                vec![decl("background-image", "url(\"../a.png\")")],
            )],
        ),
        (
            Rule::ForbiddenContent,
            vec![style(".modn-card", vec![decl("content", "\"new\"")])],
        ),
        (
            Rule::ForbiddenPositionFixed,
            vec![style(".modn-rail", vec![decl("position", "fixed")])],
        ),
        (
            Rule::ForbiddenOverlay,
            vec![style(".modn-toasts", vec![decl("z-index", "99")])],
        ),
        (
            Rule::ForbiddenImportant,
            vec![style(
                ".modn-card",
                vec![decl_at("color", "#ffffff", true, 1)],
            )],
        ),
        (
            Rule::UnknownProperty,
            vec![style(".modn-card", vec![decl("transform", "scale(1.1)")])],
        ),
        (
            Rule::ForbiddenCustomProperty,
            vec![style(".modn-app", vec![decl("--accent", "#ffffff")])],
        ),
        (
            Rule::ForbiddenSelector,
            vec![style("div .modn-card", vec![decl("color", "#ffffff")])],
        ),
        (
            Rule::UnknownHook,
            vec![style(".modn-missing", vec![decl("color", "#ffffff")])],
        ),
        (
            Rule::ProtectedRegion,
            vec![style(".modn-review", vec![decl("display", "none")])],
        ),
        (
            Rule::ProtectedColorNotOpaque,
            vec![style(
                ".modn-approval",
                vec![decl("background-color", "rgba(0, 0, 0, 0.5)")],
            )],
        ),
    ];
    for (expected, facts) in cases {
        let found = rules_of(facts);
        assert_eq!(found, vec![expected], "expected exactly {expected:?}");
    }
}

#[test]
fn documented_good_examples_are_accepted() {
    let cases = vec![
        style(".modn-card", vec![decl("color", "#ffffff")]),
        RuleFacts::Media(MediaFacts::new(
            "(max-width: 900px)",
            vec![style(".modn-card", vec![decl("display", "block")])],
            1,
        )),
        style(
            ".modn-card",
            vec![decl("background-image", "url(assets/a.png)")],
        ),
        style(".modn-card", vec![decl("opacity", "0.9")]),
        style(".modn-rail", vec![decl("position", "relative")]),
        style(".modn-rail", vec![decl("z-index", "0")]),
        style(".modn-card", vec![decl("margin", "8px")]),
        style(".modn-card", vec![decl("color", "#ffffff")]),
        style(".modn-card", vec![decl("opacity", "0.8")]),
        style(".modn-app", vec![decl("--modn-color-primary", "#ffffff")]),
        style(".modn-app .modn-card", vec![decl("color", "#ffffff")]),
        style(".modn-card", vec![decl("color", "#ffffff")]),
        style(".modn-review", vec![decl("color", "#ffffff")]),
        style(".modn-approval", vec![decl("background-color", "#000000")]),
    ];
    for facts in cases {
        let outcome = check(vec![facts]).expect("good example should be accepted");
        assert!(outcome.asset_urls.len() <= 1);
    }
}

#[test]
fn problems_carry_the_file_path_and_line() {
    let facts = vec![RuleFacts::Style(StyleFacts::new(
        vec!["div .modn-card".to_string()],
        vec![decl_at("transform", "none", false, 12)],
        10,
    ))];
    let problems = rejecting(facts);
    assert_eq!(problems.len(), 2);
    for problem in &problems {
        assert_eq!(problem.path.as_deref(), Some("theme.css"));
    }
    assert_eq!(problems[0].line, Some(10));
    assert_eq!(problems[1].line, Some(12));
}

#[test]
fn every_problem_of_a_stylesheet_is_collected() {
    let facts = vec![style(
        ".modn-card",
        vec![
            decl("color", "#ffffff"),
            decl("content", "\"x\""),
            decl("transform", "none"),
            decl_at("opacity", "1", true, 4),
        ],
    )];
    let rules = rules_of(facts);
    assert_eq!(
        rules,
        vec![
            Rule::ForbiddenContent,
            Rule::UnknownProperty,
            Rule::ForbiddenImportant
        ]
    );
}

#[test]
fn allowed_asset_urls_are_canonical_sorted_and_deduplicated() {
    let facts = vec![
        style(
            ".modn-card",
            vec![decl("background-image", "url(./assets/z.png)")],
        ),
        style(
            ".modn-tile",
            vec![
                decl("background-image", "url(\"assets/a.webp\")"),
                decl("border-image-source", "url(assets/a.webp)"),
            ],
        ),
    ];
    let outcome = check(facts).unwrap();
    assert_eq!(outcome.asset_urls, ["assets/a.webp", "assets/z.png"]);
}

#[test]
fn font_face_accepts_a_woff2_asset() {
    let facts = vec![RuleFacts::FontFace(FontFaceFacts::new(
        vec![
            decl("font-family", "\"Night City\""),
            decl("src", "url(assets/night.woff2) format(\"woff2\")"),
        ],
        1,
    ))];
    let outcome = check(facts).unwrap();
    assert_eq!(outcome.asset_urls, ["assets/night.woff2"]);
}

#[test]
fn font_face_rejects_other_extensions_properties_and_remote_urls() {
    assert_single(
        vec![RuleFacts::FontFace(FontFaceFacts::new(
            vec![decl("src", "url(assets/night.png)")],
            1,
        ))],
        Rule::UrlOutsideAssets,
    );
    assert_single(
        vec![RuleFacts::FontFace(FontFaceFacts::new(
            vec![decl("color", "#ffffff")],
            1,
        ))],
        Rule::UnknownProperty,
    );
    assert_single(
        vec![RuleFacts::FontFace(FontFaceFacts::new(
            vec![decl("src", "url(https://example.com/night.woff2)")],
            1,
        ))],
        Rule::RemoteUrl,
    );
}

#[test]
fn hostile_input_never_panics() {
    let long_selector = format!("{}{}", ".modn-card ".repeat(10_000), ".".repeat(50_000));
    let mut nested = good_style();
    for line in 0..512 {
        nested = RuleFacts::Media(MediaFacts::new("screen", vec![nested], line));
    }
    let facts = vec![
        RuleFacts::Style(StyleFacts::new(
            vec![
                String::new(),
                long_selector,
                ".modn-card\u{00e9}".to_string(),
                "\u{0000}".to_string(),
            ],
            vec![
                DeclFacts::new("", "", false, 0),
                DeclFacts::new("background-image", "url(", false, 0),
                DeclFacts::new("--modn-empty", "", false, 0),
            ],
            0,
        )),
        RuleFacts::Media(MediaFacts::new("", Vec::new(), 0)),
        RuleFacts::FontFace(FontFaceFacts::new(vec![decl("", "")], 0)),
        RuleFacts::Unsupported(UnsupportedFacts::new("", 0)),
        nested,
    ];
    let _ = check(facts);
}
