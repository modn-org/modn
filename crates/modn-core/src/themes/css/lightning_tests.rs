//! Tests for the `lightningcss`-backed theme stylesheet checker.

use crate::themes::ThemeProblem;
use crate::themes::{
    inspect_theme_dir, CheckedCss, CssChecker, CssContext, LightningChecker, Rule,
};
use std::path::{Path, PathBuf};

/// Check one stylesheet as `theme.css`.
fn check(css: &str) -> Result<CheckedCss, Vec<ThemeProblem>> {
    LightningChecker.check("theme.css", css, &CssContext::base())
}

/// Check one stylesheet and return the problems it must produce.
fn problems(css: &str) -> Vec<ThemeProblem> {
    check(css).expect_err("the stylesheet must be refused")
}

/// Assert that `css` reports `expected` on a known line.
fn assert_rule(css: &str, expected: Rule) {
    let found = problems(css);
    assert!(
        found
            .iter()
            .any(|problem| problem.rule == expected && problem.line.is_some_and(|line| line > 0)),
        "expected {expected:?} with a line, found {found:?}"
    );
}

#[test]
fn import_is_forbidden_import() {
    assert_rule("@import \"extra.css\";\n", Rule::ForbiddenImport);
}

#[test]
fn keyframes_is_forbidden_at_rule() {
    assert_rule(
        "@keyframes spin {\nfrom { opacity: 0; }\nto { opacity: 1; }\n}\n",
        Rule::ForbiddenAtRule,
    );
}

#[test]
fn supports_is_forbidden_at_rule() {
    assert_rule(
        "@supports (display: grid) {\n.modn-app { display: grid; }\n}\n",
        Rule::ForbiddenAtRule,
    );
}

#[test]
fn layer_is_forbidden_at_rule() {
    assert_rule(
        "@layer base {\n.modn-app { color: #ffffff; }\n}\n",
        Rule::ForbiddenAtRule,
    );
}

#[test]
fn namespace_is_forbidden_at_rule() {
    assert_rule(
        "@namespace svg url(http://www.w3.org/2000/svg);\n",
        Rule::ForbiddenAtRule,
    );
}

#[test]
fn container_is_forbidden_at_rule() {
    assert_rule(
        "@container (min-width: 100px) {\n.modn-app { display: grid; }\n}\n",
        Rule::ForbiddenAtRule,
    );
}

#[test]
fn nested_style_rule_is_forbidden_at_rule_as_nesting() {
    assert_rule(
        ".modn-app {\ncolor: #ffffff;\n& .modn-card { color: #ffffff; }\n}\n",
        Rule::ForbiddenAtRule,
    );
}

#[test]
fn important_is_forbidden_important() {
    assert_rule(
        ".modn-card { color: #ffffff !important; }\n",
        Rule::ForbiddenImportant,
    );
}

#[test]
fn content_is_forbidden_content() {
    assert_rule(".modn-card { content: \"new\"; }\n", Rule::ForbiddenContent);
}

#[test]
fn fixed_position_is_forbidden_position_fixed() {
    assert_rule(
        ".modn-rail { position: fixed; }\n",
        Rule::ForbiddenPositionFixed,
    );
}

#[test]
fn https_url_is_remote_url() {
    assert_rule(
        ".modn-card { background-image: url(https://x/y.png); }\n",
        Rule::RemoteUrl,
    );
}

#[test]
fn data_url_is_remote_url() {
    assert_rule(
        ".modn-card { background-image: url(data:image/png;base64,AA==); }\n",
        Rule::RemoteUrl,
    );
}

#[test]
fn absolute_url_is_remote_url() {
    assert_rule(
        ".modn-card { background-image: url(/abs.png); }\n",
        Rule::RemoteUrl,
    );
}

#[test]
fn parent_relative_url_is_url_outside_assets() {
    assert_rule(
        ".modn-card { background-image: url(../up.png); }\n",
        Rule::UrlOutsideAssets,
    );
}

#[test]
fn text_extension_is_url_outside_assets() {
    assert_rule(
        ".modn-card { background-image: url(assets/x.txt); }\n",
        Rule::UrlOutsideAssets,
    );
}

#[test]
fn transform_is_unknown_property() {
    assert_rule(
        ".modn-card { transform: scale(2); }\n",
        Rule::UnknownProperty,
    );
}

#[test]
fn universal_selector_is_forbidden_selector() {
    assert_rule("* { color: #ffffff; }\n", Rule::ForbiddenSelector);
}

#[test]
fn has_selector_is_forbidden_selector() {
    assert_rule(
        ".modn-card:has(.modn-tag) { color: #ffffff; }\n",
        Rule::ForbiddenSelector,
    );
}

#[test]
fn protected_hook_property_is_protected_region() {
    assert_rule(".modn-review { display: none; }\n", Rule::ProtectedRegion);
}

#[test]
fn protected_translucent_colour_is_protected_color_not_opaque() {
    assert_rule(
        ".modn-review { color: rgba(0, 0, 0, 0.5); }\n",
        Rule::ProtectedColorNotOpaque,
    );
}

#[test]
fn unclosed_rule_is_css_syntax_on_a_line() {
    let found = problems(".modn-card { color #ffffff; }\n");
    let problem = found
        .iter()
        .find(|problem| problem.rule == Rule::CssSyntax)
        .expect("a syntax error is reported");
    assert!(problem.line.is_some_and(|line| line > 0), "{problem:?}");
}

#[test]
fn valid_sheet_is_canonical_and_idempotent() {
    let css = concat!(
        "@font-face {\n",
        "font-family: \"Chakra Petch\";\n",
        "src: url(\"assets/x.woff2\");\n",
        "}\n",
        ".modn-main { background-image: url(\"assets/a.png\"); }\n",
        ".modn-card { background-image: url('assets/b.png'); }\n",
        ".modn-rail { background-image: url(assets/c.png); }\n",
    );
    let checked = check(css).expect("the sheet is inside the theme API");
    for path in ["a.png", "b.png", "c.png", "x.woff2"] {
        assert!(
            checked.text.contains(&format!("url(assets/{path})")),
            "canonical text holds url(assets/{path}): {}",
            checked.text
        );
    }
    assert!(!checked.text.contains("url(\""), "{}", checked.text);
    assert!(!checked.text.contains("url('"), "{}", checked.text);
    assert_eq!(
        checked.asset_urls,
        vec![
            "assets/a.png".to_owned(),
            "assets/b.png".to_owned(),
            "assets/c.png".to_owned(),
            "assets/x.woff2".to_owned(),
        ]
    );
    let again = check(&checked.text).expect("canonical text passes again");
    assert_eq!(again.text, checked.text);
    assert_eq!(again.asset_urls, checked.asset_urls);
}

#[test]
fn staged_theme_is_accepted() {
    let theme =
        inspect_theme_dir(&theme_root(), &LightningChecker).expect("the staged theme is valid");
    assert!(
        theme.css.contains("url(assets/grid.png)"),
        "canonical base CSS uses the unquoted asset form"
    );
    assert!(!theme.css.contains("url(\""));
    assert_eq!(
        theme.variant_css.keys().cloned().collect::<Vec<_>>(),
        vec!["neon".to_owned(), "noir".to_owned()]
    );
    for css in theme.variant_css.values() {
        assert!(!css.contains("url(\""), "variant canonical text: {css}");
    }
    let again = check(&theme.css).expect("canonical base CSS passes again");
    assert_eq!(again.text, theme.css);
    for (variant, css) in &theme.variant_css {
        let again = check(css).expect("canonical variant CSS passes again");
        assert_eq!(&again.text, css, "variant {variant} is idempotent");
    }
}

/// The staged official theme directory.
fn theme_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/themes/cyberpunk-2077/night-city")
}

#[test]
fn parser_message_is_cut_to_two_hundred_characters() {
    let long = "x".repeat(500);
    let cut = super::cut(&long, super::MAX_MESSAGE_CHARS);
    assert_eq!(cut.chars().count(), 200);
}
