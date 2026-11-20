use super::{Rule, ThemeProblem};
use std::collections::BTreeSet;

#[test]
fn rule_ids_match_the_documented_list() {
    let expected = [
        "forbidden-import",
        "forbidden-at-rule",
        "remote-url",
        "url-outside-assets",
        "forbidden-content",
        "forbidden-position-fixed",
        "forbidden-important",
        "unknown-property",
        "forbidden-custom-property",
        "forbidden-selector",
        "unknown-hook",
        "protected-region",
        "protected-color-not-opaque",
        "css-syntax",
        "manifest-invalid",
        "unknown-manifest-field",
        "id-mismatch",
        "missing-css",
        "missing-variant-file",
        "unexpected-file",
        "asset-type",
        "asset-too-large",
        "too-many-assets",
        "theme-too-large",
        "duplicate-theme-id",
        "digest-mismatch",
        "theme-missing",
    ];
    let rules = [
        Rule::ForbiddenImport,
        Rule::ForbiddenAtRule,
        Rule::RemoteUrl,
        Rule::UrlOutsideAssets,
        Rule::ForbiddenContent,
        Rule::ForbiddenPositionFixed,
        Rule::ForbiddenImportant,
        Rule::UnknownProperty,
        Rule::ForbiddenCustomProperty,
        Rule::ForbiddenSelector,
        Rule::UnknownHook,
        Rule::ProtectedRegion,
        Rule::ProtectedColorNotOpaque,
        Rule::CssSyntax,
        Rule::ManifestInvalid,
        Rule::UnknownManifestField,
        Rule::IdMismatch,
        Rule::MissingCss,
        Rule::MissingVariantFile,
        Rule::UnexpectedFile,
        Rule::AssetType,
        Rule::AssetTooLarge,
        Rule::TooManyAssets,
        Rule::ThemeTooLarge,
        Rule::DuplicateThemeId,
        Rule::DigestMismatch,
        Rule::ThemeMissing,
    ];
    assert_eq!(rules.len(), 27);
    assert_eq!(expected.len(), 27);
    for (rule, id) in rules.iter().zip(expected) {
        assert_eq!(rule.id(), id);
        assert_eq!(
            serde_json::to_string(rule).unwrap(),
            format!("\"{id}\""),
            "serialized id of {id}"
        );
    }
}

#[test]
fn a_problem_serialises_with_the_cli_keys() {
    let problem = ThemeProblem::new(
        Rule::UnknownProperty,
        "property \"transform\" is not allowed",
    )
    .with_theme("cyberpunk-2077/night-city")
    .with_path("theme.css")
    .with_line(12);
    let value = serde_json::to_value(&problem).unwrap();
    let object = value.as_object().unwrap();
    let keys: BTreeSet<&str> = object.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        BTreeSet::from(["line", "message", "path", "rule", "theme"])
    );
    assert_eq!(value["theme"], "cyberpunk-2077/night-city");
    assert_eq!(value["path"], "theme.css");
    assert_eq!(value["line"], 12);
    assert_eq!(value["rule"], "unknown-property");
    assert_eq!(value["message"], "property \"transform\" is not allowed");
}

#[test]
fn a_problem_without_a_location_serialises_nulls() {
    let problem = ThemeProblem::new(Rule::ManifestInvalid, "theme.json is missing");
    let value = serde_json::to_value(&problem).unwrap();
    assert!(value["theme"].is_null());
    assert!(value["path"].is_null());
    assert!(value["line"].is_null());
    assert_eq!(value["rule"], "manifest-invalid");
}

#[test]
fn display_prints_the_rule_message_and_location() {
    let located = ThemeProblem::new(Rule::UnknownProperty, "property is not allowed")
        .with_path("theme.css")
        .with_line(12);
    assert_eq!(
        located.to_string(),
        "unknown-property: property is not allowed (theme.css:12)"
    );
    let path_only = ThemeProblem::new(Rule::UnexpectedFile, "not allowed").with_path("notes.txt");
    assert_eq!(
        path_only.to_string(),
        "unexpected-file: not allowed (notes.txt)"
    );
    let bare = ThemeProblem::new(Rule::MissingCss, "theme.css is missing");
    assert_eq!(bare.to_string(), "missing-css: theme.css is missing");
}

#[test]
fn builders_set_theme_path_and_line() {
    let problem = ThemeProblem::new(Rule::AssetType, "wrong signature")
        .with_theme("game/theme")
        .with_path("assets/a.png")
        .with_line(3);
    assert_eq!(problem.theme.as_deref(), Some("game/theme"));
    assert_eq!(problem.path.as_deref(), Some("assets/a.png"));
    assert_eq!(problem.line, Some(3));
    assert_eq!(problem.rule, Rule::AssetType);
    assert_eq!(problem.message, "wrong signature");
}

#[test]
fn a_new_problem_has_no_location() {
    let problem = ThemeProblem::new(Rule::ThemeTooLarge, "too big");
    assert_eq!(problem.theme, None);
    assert_eq!(problem.path, None);
    assert_eq!(problem.line, None);
    assert_eq!(problem.rule, Rule::ThemeTooLarge);
}

#[test]
fn equality_covers_every_field() {
    let base = ThemeProblem::new(Rule::MissingCss, "missing").with_path("theme.css");
    assert_eq!(base, base.clone());
    assert_ne!(base, base.clone().with_line(1));
    assert_ne!(base, ThemeProblem::new(Rule::MissingCss, "other"));
}
