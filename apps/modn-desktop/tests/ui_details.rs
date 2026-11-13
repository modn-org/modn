use std::fs;
use std::path::{Path, PathBuf};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn read(relative: &str) -> String {
    let path = Path::new(ROOT).join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"))
}

fn details_modules() -> Vec<(PathBuf, String)> {
    let mut files = vec![(
        Path::new(ROOT).join("ui/js/details.js"),
        read("ui/js/details.js"),
    )];
    let directory = Path::new(ROOT).join("ui/js/details");
    for entry in fs::read_dir(&directory).expect("details module directory") {
        let path = entry.expect("module entry").path();
        let text = fs::read_to_string(&path).expect("details module");
        files.push((path, text));
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

fn details_js() -> String {
    details_modules()
        .into_iter()
        .map(|(_, text)| text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn relative_imports(text: &str) -> Vec<String> {
    let mut imports = Vec::new();
    let mut rest = text;
    while let Some(index) = rest.find("from '") {
        let after = &rest[index + "from '".len()..];
        let Some(end) = after.find('\'') else {
            break;
        };
        let specifier = &after[..end];
        if specifier.starts_with("./") || specifier.starts_with("../") {
            imports.push(specifier.to_string());
        }
        rest = &after[end..];
    }
    imports
}

fn var_usages(text: &str) -> Vec<String> {
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

fn has_length_unit(text: &str, unit: &str) -> bool {
    text.match_indices(unit).any(|(index, _)| {
        text[..index]
            .chars()
            .next_back()
            .is_some_and(|character| character.is_ascii_digit())
    })
}

#[test]
fn details_modules_import_resolvable_shared_code() {
    let mut reuses_operation_button = false;
    for (path, text) in details_modules() {
        let directory = path.parent().expect("module directory");
        for specifier in relative_imports(&text) {
            let target = directory.join(&specifier);
            assert!(target.is_file(), "{path:?} imports missing {specifier}");
            if specifier.ends_with("op-button.js") {
                reuses_operation_button = true;
            }
        }
    }
    assert!(
        reuses_operation_button,
        "Mod details must reuse the Library operation button"
    );
    let details = read("ui/js/details.js");
    for shared in ["./api.js", "./messages.js", "./toast.js"] {
        assert!(
            details.contains(&format!("from '{shared}'")),
            "details.js must import {shared}"
        );
    }
}

#[test]
fn the_file_plan_loads_only_when_the_details_section_opens() {
    let modules = details_modules();
    let readers: Vec<&(PathBuf, String)> = modules
        .iter()
        .filter(|(_, text)| text.contains("planInstall("))
        .collect();
    assert_eq!(readers.len(), 1, "exactly one module reads the file plan");
    let (path, source) = readers[0];
    assert_eq!(
        source.matches("planInstall(").count(),
        1,
        "{path:?} must call planInstall once"
    );
    let call = source.find("planInstall(").expect("planInstall call");
    let head = &source[..call];
    let definition = head
        .rfind("function ")
        .expect("planInstall is inside a function");
    let name: String = head[definition + "function ".len()..]
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
        .collect();
    assert_eq!(name, "loadPlan", "{path:?} must read the plan in loadPlan");
    let callers = source
        .match_indices("loadPlan(")
        .filter(|(index, _)| !source[..*index].ends_with("function "))
        .count();
    assert_eq!(
        callers, 1,
        "{path:?} must call loadPlan only from the toggle listener"
    );
    let listener = source
        .find("addEventListener('toggle'")
        .expect("the plan section listens for toggle");
    let listener_end = listener + source[listener..].find("});").expect("listener body end");
    let body = &source[listener..listener_end];
    assert!(
        body.contains("loadPlan()"),
        "{path:?} toggle listener must load the plan"
    );
    assert!(
        body.contains("section.open"),
        "{path:?} must read the plan only when the section opens"
    );
}

#[test]
fn details_wire_the_commands_and_the_clipboard() {
    let all = details_js();
    for call in [
        "catalog(",
        "listPackages(",
        "history(",
        "undo(",
        "diagnostics(",
        "installMod(",
        "uninstall(",
        "update(",
    ] {
        assert!(all.contains(call), "Mod details must call {call}");
    }
    assert!(
        all.contains("navigator.clipboard.writeText("),
        "Copy diagnostics must write the report to the clipboard"
    );
    assert!(
        all.contains("showToast('Diagnostics copied.')"),
        "Copy diagnostics must confirm success"
    );
    assert!(
        all.contains("createOperationButton("),
        "the main action and Undo reuse the operation button"
    );
    let details = read("ui/js/details.js");
    assert!(
        details.contains("#/game/") && details.contains("?path="),
        "the back link must return to the Library route"
    );
}

#[test]
fn details_expose_the_verifier_hooks() {
    let all = details_js();
    for hook in [
        "details-back",
        "details-plan",
        "details-history",
        "history-undo",
        "copy-diagnostics",
    ] {
        assert!(
            all.contains(&format!("'{hook}'")),
            "missing data-testid {hook}"
        );
    }
    assert!(
        all.contains("createElement('details')"),
        "the file plan must be a collapsed details section"
    );
    for accessible in [
        "aria-label', `Back to the",
        "aria-label', 'File plan'",
        "aria-label', 'Operation history'",
        "aria-label', 'Copy diagnostics'",
    ] {
        assert!(
            all.contains(accessible),
            "missing accessible name {accessible}"
        );
    }
}

#[test]
fn details_never_show_raw_cli_output() {
    let all = details_js();
    for raw in ["blocker.message", "error.message", "response.message"] {
        assert!(
            !all.contains(raw),
            "Mod details must not render {raw}; use messages.js"
        );
    }
    assert!(
        all.contains("messageFor("),
        "failures must use the shared message table"
    );
}

#[test]
fn details_css_uses_only_defined_tokens() {
    let tokens = read("ui/css/tokens.css");
    let mut defined = Vec::new();
    for line in tokens.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("--modn-")
            && let Some((name, _)) = rest.split_once(':')
        {
            defined.push(format!("--modn-{name}"));
        }
    }
    assert!(!defined.is_empty(), "tokens.css defines no variables");
    let css = read("ui/css/details.css");
    for usage in var_usages(&css) {
        assert!(
            defined.iter().any(|token| token == &usage),
            "details.css uses undefined {usage}"
        );
    }
    for literal in ["#", "rgb(", "hsl("] {
        assert!(
            !css.contains(literal),
            "details.css must use tokens instead of {literal}"
        );
    }
    for unit in ["px", "rem", "em"] {
        assert!(
            !has_length_unit(&css, unit),
            "details.css must use tokens instead of a {unit} length"
        );
    }
    for rule in [
        ".modn-details__summary",
        ".modn-plan__row",
        ".modn-history__entry",
    ] {
        assert!(css.contains(rule), "details.css is missing {rule}");
    }
}

#[test]
fn details_files_stay_within_the_size_limit() {
    let mut files = vec![
        Path::new(ROOT).join("ui/js/details.js"),
        Path::new(ROOT).join("ui/css/details.css"),
        Path::new(ROOT).join("tests/ui_details.rs"),
    ];
    for (path, _) in details_modules() {
        files.push(path);
    }
    for path in files {
        let text = fs::read_to_string(&path).expect("owned file");
        let lines = text.lines().count();
        assert!(lines <= 500, "{path:?} has {lines} lines");
    }
}
