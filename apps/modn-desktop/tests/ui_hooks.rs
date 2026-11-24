//! Keeps the theme API hook tables in `docs/themes.md` and the desktop UI in
//! step.
//!
//! The document is the contract a theme author reads. A hook that the doc
//! names must exist in `ui/`, and a `.modn-*` class that the UI styles must be
//! named by the doc.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn read(relative: &str) -> String {
    let path = Path::new(ROOT).join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"))
}

/// Returns the document that holds the stable hook tables.
fn themes_doc() -> String {
    read("../../docs/themes.md")
}

fn ui_files() -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    collect_ui_files(&Path::new(ROOT).join("ui"), &mut files);
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

fn collect_ui_files(directory: &Path, files: &mut Vec<(PathBuf, String)>) {
    let entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("cannot list {directory:?}: {error}"));
    for entry in entries {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            collect_ui_files(&path, files);
        } else {
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"));
            files.push((path, text));
        }
    }
}

/// Returns the data rows of the first Markdown table under `heading`.
///
/// The header row and the `---` separator are skipped, so a caller only sees
/// cells of real entries. A blank or prose line ends the table.
fn table_rows(document: &str, heading: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut in_section = false;
    let mut saw_header = false;
    for line in document.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            in_section = line == heading;
            saw_header = false;
            continue;
        }
        if !in_section {
            continue;
        }
        if !line.starts_with('|') {
            if saw_header && !rows.is_empty() {
                break;
            }
            continue;
        }
        let cells: Vec<String> = line
            .trim_matches('|')
            .split('|')
            .map(|cell| cell.trim().to_string())
            .collect();
        if !saw_header {
            saw_header = true;
            continue;
        }
        if cells
            .iter()
            .all(|cell| !cell.is_empty() && cell.chars().all(|ch| ch == '-' || ch == ':'))
        {
            continue;
        }
        rows.push(cells);
    }
    rows
}

/// Returns the hook name in a table's first cell, without Markdown decoration.
fn cell_name(cell: &str) -> String {
    cell.trim()
        .trim_matches('`')
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string()
}

fn first_cells(rows: Vec<Vec<String>>) -> Vec<String> {
    rows.iter()
        .filter_map(|cells| cells.first())
        .map(|cell| cell_name(cell))
        .collect()
}

fn documented_classes(document: &str) -> Vec<String> {
    first_cells(table_rows(document, "### Classes"))
        .into_iter()
        .map(|name| name.trim_start_matches('.').to_string())
        .collect()
}

fn documented_attributes(document: &str) -> Vec<String> {
    first_cells(table_rows(document, "### Data attributes"))
}

fn documented_properties(document: &str) -> Vec<String> {
    first_cells(table_rows(document, "### Custom properties"))
}

fn documented_protected(document: &str) -> Vec<String> {
    first_cells(table_rows(document, "### Protected hooks"))
}

/// Collects every `.modn-*` class name that has a rule in a stylesheet.
fn css_classes() -> BTreeSet<String> {
    let mut classes = BTreeSet::new();
    for (path, text) in ui_files() {
        if !path.extension().is_some_and(|extension| extension == "css") {
            continue;
        }
        let mut rest = text.as_str();
        while let Some(index) = rest.find(".modn-") {
            let after = &rest[index + 1..];
            let name: String = after
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
                .collect();
            rest = &after[name.len()..];
            if !name.is_empty() {
                classes.insert(name);
            }
        }
    }
    classes
}

/// Turns `data-game-id` into the `gameId` property name that `dataset` uses.
fn camel_case(kebab: &str) -> String {
    let mut camel = String::new();
    for (index, part) in kebab.split('-').enumerate() {
        if index == 0 {
            camel.push_str(part);
        } else {
            let mut characters = part.chars();
            if let Some(first) = characters.next() {
                camel.extend(first.to_uppercase());
                camel.push_str(characters.as_str());
            }
        }
    }
    camel
}

/// Reports whether any UI file writes the data attribute.
fn attribute_appears(attribute: &str, ui: &str) -> bool {
    let kebab = attribute.strip_prefix("data-").unwrap_or(attribute);
    let camel = camel_case(kebab);
    ui.contains(&format!("data-{kebab}"))
        || ui.contains(&format!("dataset.{camel}"))
        || ui.contains(&format!("['{camel}']"))
}

fn tokens_define(tokens: &str, property: &str) -> bool {
    tokens
        .lines()
        .map(str::trim)
        .any(|line| line.starts_with(&format!("{property}:")))
}

#[test]
fn documented_hooks_exist_in_the_ui() {
    let document = themes_doc();
    let classes = css_classes();
    let ui: String = ui_files()
        .into_iter()
        .map(|(_, text)| text)
        .collect::<Vec<_>>()
        .join("\n");
    let tokens = read("ui/css/tokens.css");

    let mut missing = Vec::new();
    for class in documented_classes(&document) {
        if !classes.contains(&class) {
            missing.push(format!(
                "class .{class} is in docs/themes.md but has no rule in ui/css"
            ));
        }
    }
    for attribute in documented_attributes(&document) {
        if !attribute_appears(&attribute, &ui) {
            missing.push(format!(
                "attribute [{attribute}] is in docs/themes.md but not in ui/"
            ));
        }
    }
    for property in documented_properties(&document) {
        if !tokens_define(&tokens, &property) {
            missing.push(format!(
                "property {property} is in docs/themes.md but not defined in ui/css/tokens.css"
            ));
        }
    }
    assert!(
        missing.is_empty(),
        "hooks missing in UI ({}):\n{}",
        missing.len(),
        missing.join("\n")
    );
}

#[test]
fn every_styled_class_is_documented() {
    let document = themes_doc();
    let documented: BTreeSet<String> = documented_classes(&document).into_iter().collect();
    let missing: Vec<String> = css_classes()
        .into_iter()
        .filter(|class| !documented.contains(class))
        .map(|class| format!(".{class} has a rule in ui/css but is missing in docs/themes.md"))
        .collect();
    assert!(
        missing.is_empty(),
        "classes missing in doc ({}):\n{}",
        missing.len(),
        missing.join("\n")
    );
}

#[test]
fn protected_hooks_use_the_reserved_prefixes() {
    let document = themes_doc();
    let classes = documented_protected(&document);
    assert!(!classes.is_empty(), "the doc lists no protected hooks");
    for class in &classes {
        assert!(
            class.starts_with(".modn-review") || class.starts_with(".modn-approval"),
            "protected hook {class} must start with .modn-review or .modn-approval"
        );
    }
}

#[test]
fn reserved_hooks_stay_inside_their_regions() {
    for (path, text) in ui_files() {
        let relative = path
            .strip_prefix(Path::new(ROOT).join("ui"))
            .expect("a UI path")
            .to_string_lossy()
            .replace('\\', "/");
        if text.contains("modn-review") {
            assert!(
                [
                    "js/details/plan.js",
                    "css/details.css",
                    "css/base.css",
                    "css/protected.css"
                ]
                .contains(&relative.as_str()),
                "{relative} uses the install-review hooks outside their region"
            );
        }
        if text.contains("modn-approval") {
            assert!(
                ["js/library/ask.js", "css/library.css", "css/protected.css"]
                    .contains(&relative.as_str()),
                "{relative} uses the approval hooks outside their region"
            );
        }
    }
}

/// Returns the `PROTECTED_CONTEXT_CLASSES` list from the core hook table.
///
/// The list is read from `crates/modn-core/src/themes/hooks.rs`, because the
/// desktop crate does not depend on the core crate.
fn context_classes_in_code() -> Vec<String> {
    const DECLARATION: &str = "pub const PROTECTED_CONTEXT_CLASSES: &[&str] = &[";
    let source = read("../../crates/modn-core/src/themes/hooks.rs");
    let start = source
        .find(DECLARATION)
        .expect("hooks.rs declares PROTECTED_CONTEXT_CLASSES")
        + DECLARATION.len();
    let rest = &source[start..];
    let end = rest.find("];").expect("the context class list ends");
    rest[..end]
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

/// One element that holds or surrounds a protected region.
///
/// The first cell is the class string the element carries, the second is the
/// UI file that builds it, and the third is the source text that proves it.
/// A new class in one of these strings, or a changed string, fails the test in
/// `every_context_element_is_a_context_hook`.
const PROTECTED_CONTEXT_ELEMENTS: &[(&str, &str, &str)] = &[
    ("modn-app", "ui/index.html", "class=\"modn-app\""),
    ("modn-main", "ui/index.html", "class=\"modn-main\""),
    (
        "modn-view modn-details",
        "ui/js/details.js",
        "'modn-view modn-details'",
    ),
    (
        "modn-details__ask",
        "ui/js/details.js",
        "'modn-details__ask'",
    ),
    (
        "modn-view modn-library",
        "ui/js/library.js",
        "'modn-view modn-library'",
    ),
    (
        "modn-library__ask",
        "ui/js/library.js",
        "'modn-library__ask'",
    ),
    ("modn-recent", "ui/js/library/recent.js", "'modn-recent'"),
    (
        "modn-recent__menu",
        "ui/js/library/recent.js",
        "'modn-recent__menu'",
    ),
    (
        "modn-recent__entry",
        "ui/js/library/recent.js",
        "'modn-recent__entry'",
    ),
    (
        "modn-recent__text",
        "ui/js/library/recent.js",
        "'modn-recent__text'",
    ),
];

#[test]
fn every_context_element_is_a_context_hook() {
    let index = read("ui/index.html");
    assert!(
        index.contains("<main class=\"modn-main\" id=\"modn-view\""),
        "the router mounts every view inside .modn-main"
    );
    assert!(
        read("ui/js/app.js").contains("getElementById('modn-view')"),
        "app.js mounts the active view into #modn-view"
    );

    let context = context_classes_in_code();
    assert!(!context.is_empty(), "hooks.rs lists no context hook");
    let mut missing = Vec::new();
    for (classes, file, needle) in PROTECTED_CONTEXT_ELEMENTS {
        let source = read(file);
        if !source.contains(needle) {
            missing.push(format!("{file} no longer builds {needle}"));
            continue;
        }
        for class in classes.split(' ') {
            let reserved = class.starts_with("modn-review") || class.starts_with("modn-approval");
            if !reserved && !context.iter().any(|known| known == class) {
                missing.push(format!(
                    ".{class} in {file} holds a protected region but is not a context hook"
                ));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "context hook problems ({}):\n{}",
        missing.len(),
        missing.join("\n")
    );
}
