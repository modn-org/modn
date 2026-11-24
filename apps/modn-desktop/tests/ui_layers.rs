//! Guards the cascade-layer split that keeps the protected regions above
//! theme CSS.
//!
//! The desktop stylesheets live in one lower `app` layer, theme stylesheets are
//! wrapped in a `theme` layer at apply time, and the install review and the
//! approval prompt put their safety-critical properties in a `protected`
//! layer that comes last.
use std::fs;
use std::path::{Path, PathBuf};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
const LAYER_ORDER: &str = "@layer app, theme, protected;";

fn read(relative: &str) -> String {
    let path = Path::new(ROOT).join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"))
}

fn css_files() -> Vec<(String, String)> {
    let directory = Path::new(ROOT).join("ui/css");
    let mut files = Vec::new();
    for entry in fs::read_dir(&directory).expect("css directory") {
        let path: PathBuf = entry.expect("css entry").path();
        if path.extension().is_some_and(|extension| extension == "css") {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("css file name")
                .to_string();
            let text = fs::read_to_string(&path).expect("css file");
            files.push((name, text));
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

fn stylesheet(name: &str) -> String {
    css_files()
        .into_iter()
        .find(|(file, _)| file == name)
        .map(|(_, text)| text)
        .unwrap_or_else(|| panic!("missing ui/css/{name}"))
}

/// Removes CSS comments so that statements and rules compare by content.
fn strip_comments(css: &str) -> String {
    let mut output = String::new();
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        output.push_str(&rest[..start]);
        let Some(end) = rest[start + 2..].find("*/") else {
            return output;
        };
        rest = &rest[start + 2 + end + 2..];
    }
    output.push_str(rest);
    output
}

/// Returns the body of `@layer <name> { ... }` and asserts that nothing
/// follows the closing brace.
fn layer_body<'a>(css: &'a str, name: &str) -> &'a str {
    let trimmed = css.trim();
    let header = format!("@layer {name} {{");
    let body = trimmed.strip_prefix(header.as_str()).unwrap_or_else(|| {
        panic!("the stylesheet must start with `{header}`: {trimmed:.80}")
    });
    let mut depth = 1usize;
    for (index, character) in body.char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    assert!(
                        body[index + 1..].trim().is_empty(),
                        "`@layer {name}` must be the whole stylesheet"
                    );
                    return &body[..index];
                }
            }
            _ => {}
        }
    }
    panic!("unterminated `@layer {name}` block");
}

/// One selector and its declaration block from a stylesheet body.
struct Rule {
    selector: String,
    declarations: Vec<(String, String)>,
}

/// Reads the simple rules of a stylesheet body. The UI stylesheets have no
/// nested at-rules, so one selector before a brace is enough.
fn rules(body: &str) -> Vec<Rule> {
    let mut rules = Vec::new();
    let mut rest = body;
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return rules;
        }
        let Some(open) = rest.find('{') else {
            panic!("a rule has no opening brace: {rest:.80}");
        };
        let selector = rest[..open].trim().to_string();
        assert!(!selector.is_empty(), "a rule has no selector");
        let after = &rest[open + 1..];
        let mut depth = 1usize;
        let mut close = None;
        for (index, character) in after.char_indices() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(index);
                        break;
                    }
                }
                _ => {}
            }
        }
        let close = close.unwrap_or_else(|| panic!("unterminated rule {selector}"));
        let declarations = after[..close]
            .split(';')
            .filter_map(|declaration| {
                let declaration = declaration.trim();
                if declaration.is_empty() {
                    return None;
                }
                let (property, value) = declaration
                    .split_once(':')
                    .unwrap_or_else(|| panic!("malformed declaration {declaration}"));
                Some((property.trim().to_ascii_lowercase(), value.trim().to_string()))
            })
            .collect();
        rules.push(Rule {
            selector,
            declarations,
        });
        rest = &after[close + 1..];
    }
}

fn protected_hook(selector: &str) -> bool {
    selector.contains(".modn-review")
        || selector.contains(".modn-approval")
        || selector.contains("[data-modn-protected")
}

/// The properties whose value a theme must not be able to change inside a
/// protected region.
fn safety_critical(property: &str) -> bool {
    const NAMES: [&str; 25] = [
        "display",
        "visibility",
        "opacity",
        "position",
        "overflow",
        "clip-path",
        "width",
        "height",
        "min-width",
        "max-width",
        "min-height",
        "max-height",
        "margin",
        "padding",
        "gap",
        "font-family",
        "font-size",
        "font-weight",
        "line-height",
        "letter-spacing",
        "text-indent",
        "white-space",
        "text-overflow",
        "z-index",
        "pointer-events",
    ];
    NAMES.contains(&property)
        || property.starts_with("margin-")
        || property.starts_with("padding-")
        || property == "row-gap"
        || property == "column-gap"
        || property == "overflow-x"
        || property == "overflow-y"
}

fn app_body(css: &str, name: &str) -> String {
    let cleaned = strip_comments(css);
    let mut rest = cleaned.trim();
    if name == "tokens.css" {
        rest = rest.strip_prefix(LAYER_ORDER).unwrap_or_else(|| {
            panic!("tokens.css must start with `{LAYER_ORDER}`")
        });
        rest = rest.trim();
    }
    layer_body(rest, "app").to_string()
}

#[test]
fn the_first_loaded_stylesheet_declares_the_layer_order() {
    let sheets = {
        let html = read("ui/index.html");
        let mut sheets = Vec::new();
        let mut rest = html.as_str();
        while let Some(index) = rest.find("href=\"css/") {
            let after = &rest[index + "href=\"css/".len()..];
            let Some(end) = after.find('"') else {
                break;
            };
            sheets.push(after[..end].to_string());
            rest = &after[end..];
        }
        sheets
    };
    assert_eq!(
        sheets.first().map(String::as_str),
        Some("tokens.css"),
        "tokens.css must be the first stylesheet index.html loads"
    );
    assert_eq!(
        sheets.last().map(String::as_str),
        Some("protected.css"),
        "protected.css must load after every other stylesheet"
    );
    let cleaned = strip_comments(&stylesheet("tokens.css"));
    let first = cleaned.trim_start();
    assert!(
        first.starts_with(LAYER_ORDER),
        "the first non-comment statement of tokens.css must be `{LAYER_ORDER}`"
    );
    assert_eq!(
        first.matches("@layer app, theme, protected;").count(),
        1,
        "the layer order statement must appear once"
    );
}

#[test]
fn every_app_stylesheet_is_one_app_layer() {
    let files = css_files();
    assert!(
        files.len() >= 7,
        "the UI must ship tokens, base, home, library, details, theme and protected"
    );
    for (name, text) in &files {
        if name == "protected.css" {
            continue;
        }
        let body = app_body(text, name);
        assert!(
            !body.contains("@layer "),
            "{name} must not nest another layer inside the app layer"
        );
        let mut saw_rule = false;
        for rule in rules(&body) {
            saw_rule = true;
            assert!(
                !rule.selector.starts_with('@'),
                "{name} must hold plain rules inside the app layer"
            );
        }
        assert!(saw_rule, "{name} must hold at least one rule");
    }
}

#[test]
fn the_protected_stylesheet_is_one_protected_layer() {
    let cleaned = strip_comments(&stylesheet("protected.css"));
    let body = layer_body(&cleaned, "protected");
    assert!(!body.contains("@layer "), "protected.css must not nest layers");
    assert!(!rules(body).is_empty(), "protected.css must hold rules");
}

#[test]
fn the_protected_layer_uses_literal_safety_values_only() {
    let css = stylesheet("protected.css");
    assert!(
        !css.contains("var("),
        "protected.css must use literal values, not custom properties"
    );
    let cleaned = strip_comments(&css);
    let body = layer_body(&cleaned, "protected");
    for rule in rules(body) {
        assert!(
            protected_hook(&rule.selector),
            "protected.css styles {} outside a protected hook",
            rule.selector
        );
        for (property, _) in &rule.declarations {
            assert!(
                safety_critical(property),
                "protected.css must hold only safety-critical properties, found {property} in {}",
                rule.selector
            );
        }
    }
}

#[test]
fn app_stylesheets_leave_safety_properties_to_the_protected_layer() {
    for (name, text) in css_files() {
        if name == "protected.css" {
            continue;
        }
        let body = app_body(&text, &name);
        for rule in rules(&body) {
            if !protected_hook(&rule.selector) {
                continue;
            }
            for (property, _) in &rule.declarations {
                assert!(
                    !safety_critical(property),
                    "{name} sets {property} on {}; move it to protected.css",
                    rule.selector
                );
            }
        }
    }
}

#[test]
fn protected_stylesheet_covers_every_moved_hook() {
    let cleaned = strip_comments(&stylesheet("protected.css"));
    let body = layer_body(&cleaned, "protected");
    let selectors: Vec<String> = rules(body)
        .into_iter()
        .map(|rule| rule.selector.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();
    for required in [
        ".modn-review",
        ".modn-review__summary",
        ".modn-review[open] .modn-review__summary",
        ".modn-review__body",
        ".modn-review__muted",
        ".modn-review__status",
        ".modn-review__list",
        ".modn-review__row",
        ".modn-review__path",
        ".modn-review__tag",
        ".modn-review__blocker",
        ".modn-approval",
        ".modn-approval__question",
        ".modn-approval__list",
        ".modn-approval__path",
        ".modn-approval__actions",
        ".modn-approval__button",
        ".modn-approval__button:disabled",
        ".modn-approval__button--quiet",
    ] {
        let needle = required.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            selectors.iter().any(|selector| selector == &needle),
            "protected.css must hold a safety rule for {required}"
        );
    }
}

#[test]
fn layer_files_stay_within_the_size_limit() {
    for (name, text) in css_files() {
        let lines = text.lines().count();
        assert!(lines <= 500, "ui/css/{name} has {lines} lines");
    }
    let test = read("tests/ui_layers.rs");
    assert!(test.lines().count() <= 500, "tests/ui_layers.rs is too long");
}
