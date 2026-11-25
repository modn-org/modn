use std::fs;
use std::path::Path;
const ROOT: &str = env!("CARGO_MANIFEST_DIR");
const THEME_FILES: [&str; 6] = [
    "ui/js/theme/loader.js",
    "ui/js/theme/controller.js",
    "ui/js/theme/panel.js",
    "ui/js/app.js",
    "ui/css/theme.css",
    "ui/index.html",
];
const SHARED_CLASSES: [&str; 3] = ["modn-button", "modn-button--quiet", "modn-select"];
fn read(relative: &str) -> String {
    let path = Path::new(ROOT).join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"))
}
fn class_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut rest = text;
    while let Some(index) = rest.find("modn-") {
        let after = &rest[index..];
        let token: String = after
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
            .collect();
        rest = &after[token.len()..];
        if !token.is_empty() && !tokens.contains(&token) {
            tokens.push(token);
        }
    }
    tokens
}
fn styled_classes(text: &str) -> Vec<String> {
    let mut classes = Vec::new();
    let mut rest = text;
    while let Some(index) = rest.find(".modn-") {
        let after = &rest[index + 1..];
        let name: String = after
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
            .collect();
        rest = &after[name.len()..];
        if !name.is_empty() && !classes.contains(&name) {
            classes.push(name);
        }
    }
    classes
}
#[test]
fn theme_ui_avoids_markup_injection_dynamic_code_and_inline_styling() {
    let banned = [
        "innerHTML",
        "outerHTML",
        "insertAdjacentHTML",
        "eval(",
        "new Function",
    ];
    for relative in THEME_FILES {
        let text = read(relative);
        for pattern in banned {
            assert!(!text.contains(pattern), "{relative} must not use {pattern}");
        }
        let lower = text.to_ascii_lowercase();
        assert!(!lower.contains("style="), "{relative} must not use style=");
        assert!(!lower.contains("<style"), "{relative} must not use <style");
    }
}
#[test]
fn index_links_the_theme_stylesheet_last() {
    let html = read("ui/index.html");
    let theme_link = "<link rel=\"stylesheet\" href=\"css/theme.css\">";
    let position = html
        .find(theme_link)
        .unwrap_or_else(|| panic!("index.html must link {theme_link}"));
    for earlier in [
        "css/tokens.css",
        "css/base.css",
        "css/home.css",
        "css/library.css",
        "css/details.css",
    ] {
        let other = html
            .find(earlier)
            .unwrap_or_else(|| panic!("index.html must link {earlier}"));
        assert!(
            other < position,
            "theme.css must come after {earlier} so the app sheets apply first"
        );
    }
}
#[test]
fn every_new_class_uses_the_reserved_theme_prefix() {
    for class in styled_classes(&read("ui/css/theme.css")) {
        assert!(
            class.starts_with("modn-theme-"),
            "theme.css styles .{class}, which is outside the reserved prefix"
        );
    }
    for token in class_tokens(&read("ui/js/theme/panel.js")) {
        if SHARED_CLASSES.contains(&token.as_str()) {
            continue;
        }
        assert!(
            token.starts_with("modn-theme"),
            "panel.js uses {token}, which is outside the reserved prefix"
        );
    }
    let html = read("ui/index.html");
    assert!(
        html.contains("class=\"modn-theme-shell\""),
        "the shell header must use the reserved prefix"
    );
}
#[test]
fn each_route_loads_the_theme_without_blocking_the_screen() {
    let app = read("ui/js/app.js");
    assert!(
        app.contains("loadThemeForGame(gameId)"),
        "a route must load the theme of its game"
    );
    assert!(
        !app.contains("await loadThemeForGame"),
        "the theme load must not block the route"
    );
    let theme_call = app
        .find("applyThemeFor(ctx.gameId")
        .expect("a route must point the theme control at its game");
    let render = app
        .find("await module.render")
        .expect("a route renders its module");
    assert!(
        theme_call < render,
        "the theme load must start before the screen waits on its data"
    );
}
