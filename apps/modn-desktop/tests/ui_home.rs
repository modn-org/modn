//! Static checks for the Home view of the desktop UI.
//!
//! The app ships plain ES modules and no bundler, so these tests hold the
//! promises the browser and the verifier rely on: close imports, the stable
//! test ids, and a stylesheet built from design tokens only.

use std::fs;
use std::path::{Path, PathBuf};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn read(relative: &str) -> String {
    let path = Path::new(ROOT).join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"))
}

fn collect_files(directory: &Path, files: &mut Vec<(PathBuf, String)>) {
    let entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("cannot list {directory:?}: {error}"));
    for entry in entries {
        let entry = entry.expect("directory entry");
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, files);
        } else {
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"));
            files.push((path, text));
        }
    }
}

/// `ui/js/home.js` plus every module under `ui/js/home/`.
fn home_sources() -> Vec<(PathBuf, String)> {
    let mut files = vec![(Path::new(ROOT).join("ui/js/home.js"), read("ui/js/home.js"))];
    let mut modules = Vec::new();
    collect_files(&Path::new(ROOT).join("ui/js/home"), &mut modules);
    modules.sort_by(|left, right| left.0.cmp(&right.0));
    files.extend(modules);
    files
}

fn home_script() -> String {
    home_sources()
        .into_iter()
        .map(|(_, text)| text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn import_specifiers(text: &str) -> Vec<String> {
    let mut specifiers = Vec::new();
    let mut rest = text;
    while let Some(index) = rest.find("from '") {
        let after = &rest[index + "from '".len()..];
        let Some(end) = after.find('\'') else {
            break;
        };
        specifiers.push(after[..end].to_string());
        rest = &after[end..];
    }
    specifiers
}

fn token_definitions() -> Vec<String> {
    let tokens = read("ui/css/tokens.css");
    let mut definitions = Vec::new();
    for line in tokens.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("--modn-")
            && let Some((name, _)) = rest.split_once(':')
        {
            definitions.push(format!("--modn-{name}"));
        }
    }
    assert!(!definitions.is_empty(), "tokens.css defines no variables");
    definitions
}

fn token_usages(text: &str) -> Vec<String> {
    let mut usages = Vec::new();
    let mut rest = text;
    while let Some(index) = rest.find("var(--modn-") {
        let after = &rest[index + "var(".len()..];
        let Some(end) = after.find(')') else {
            break;
        };
        usages.push(after[..end].to_string());
        rest = &after[end..];
    }
    usages
}

fn home_css() -> String {
    read("ui/css/home.css")
}

/// Returns the source from `marker` through its first balanced brace block.
fn braced<'a>(source: &'a str, marker: &str) -> &'a str {
    let start = source
        .find(marker)
        .unwrap_or_else(|| panic!("missing {marker}"));
    let open = start + source[start..].find('{').expect("opening brace");
    let mut depth = 0usize;
    for (offset, character) in source[open..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[start..open + offset + 1];
                }
            }
            _ => {}
        }
    }
    panic!("unterminated {marker}");
}

fn rule_declares(css: &str, selector: &str, declaration: &str) -> bool {
    css.split('}')
        .any(|block| block.contains(selector) && block.contains(declaration))
}

#[test]
fn home_imports_only_existing_modules() {
    for (path, text) in home_sources() {
        let directory = path.parent().expect("javascript file has a directory");
        for specifier in import_specifiers(&text) {
            assert!(
                specifier.starts_with("./") || specifier.starts_with("../"),
                "{path:?} imports non-relative {specifier}"
            );
            let target = directory.join(&specifier);
            assert!(target.is_file(), "{path:?} imports missing {specifier}");
        }
    }
}

#[test]
fn home_detects_and_activates_games() {
    let source = home_script();
    for call in [
        "listGames()",
        "detectGames()",
        "activateGame(",
        "addGame(",
        "removeGame(",
    ] {
        assert!(source.contains(call), "home view never calls {call}");
    }
    let home = read("ui/js/home.js");
    assert!(
        home.contains("activateGame({"),
        "home.js must activate a detected game itself"
    );
}

#[test]
fn home_shows_registered_and_detected_cards() {
    let source = home_script();
    assert!(
        source.contains("card.dataset.state = entry.state"),
        "cards must publish their state as data-state"
    );
    assert!(
        source.contains("state: 'registered'"),
        "no registered state"
    );
    assert!(source.contains("state: 'detected'"), "no detected state");
    assert!(
        source.contains("`game-card-${entry.gameId}`"),
        "cards must carry the per-game test id"
    );
}

#[test]
fn home_exposes_the_verifier_test_ids() {
    let source = home_script();
    for test_id in [
        "home-scan",
        "home-add",
        "game-card-remove",
        "game-card-error",
        "home-empty",
    ] {
        assert!(
            source.contains(&format!("'{test_id}'")),
            "home view has no {test_id} test id"
        );
    }
}

#[test]
fn home_toolbar_offers_scan_add_and_remove() {
    let source = home_script();
    assert!(source.contains("'Scan'"), "no Scan control");
    assert!(source.contains("'Add'"), "no Add control");
    assert!(
        source.contains("'Remove from Modn'"),
        "no Remove from Modn control"
    );
    assert!(
        source.contains("canRemove"),
        "Remove from Modn must depend on an empty package list"
    );
}

#[test]
fn add_always_offers_every_supported_game() {
    let data = read("ui/js/home/data.js");
    let function = braced(&data, "export function supportedGameIds(entries)");
    assert!(
        function.contains("new Set(FALLBACK_GAME_IDS)"),
        "Add must offer the supported games even when a card already exists"
    );
    assert!(
        !function.contains("ids.size === 0"),
        "Add must not offer the supported games only when the grid is empty"
    );
    assert!(
        function.contains("[...ids].sort()"),
        "Add must list every offered game once"
    );
}

#[test]
fn home_css_uses_only_defined_tokens() {
    let definitions = token_definitions();
    let css = home_css();
    for usage in token_usages(&css) {
        assert!(
            definitions.iter().any(|definition| definition == &usage),
            "home.css uses undefined {usage}"
        );
    }
    assert!(!css.contains('#'), "home.css holds a raw colour");
    assert!(!css.contains("rgb("), "home.css holds a raw colour");
    assert!(!css.contains("hsl("), "home.css holds a raw colour");
    let without_hairlines = css.replace("1px", "");
    assert!(
        !without_hairlines.contains("px"),
        "home.css holds a raw length beyond a 1px hairline"
    );
}

#[test]
fn home_css_grids_cards_at_full_width() {
    let css = home_css();
    assert!(
        css.contains("repeat(auto-fill, minmax(calc(var(--modn-space-"),
        "the card grid must size itself from a space token"
    );
    assert!(css.contains("1fr)"), "the card grid must fill the width");
    assert!(
        rule_declares(&css, ".modn-game-card", "height: 100%"),
        "cards must share one height"
    );
}

#[test]
fn home_css_grays_out_detected_cards() {
    let css = home_css();
    let selector = "[data-state=\"detected\"]";
    assert!(
        rule_declares(&css, selector, "filter: grayscale(1)"),
        "detected cards must be grayscale"
    );
    assert!(
        rule_declares(&css, selector, "opacity: var(--modn-opacity-disabled)"),
        "detected cards must fade through a token"
    );
}

#[test]
fn home_files_stay_small() {
    for (path, text) in home_sources() {
        let lines = text.lines().count();
        assert!(lines <= 500, "{path:?} has {lines} lines");
    }
    let css_lines = home_css().lines().count();
    assert!(css_lines <= 500, "ui/css/home.css has {css_lines} lines");
}
