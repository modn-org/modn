use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src ipc: http://ipc.localhost; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'";

fn read(relative: &str) -> String {
    let path = Path::new(ROOT).join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"))
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
        let entry = entry.expect("directory entry");
        let path = entry.path();
        if path.is_dir() {
            collect_ui_files(&path, files);
        } else {
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"));
            files.push((path, text));
        }
    }
}

fn css_files() -> Vec<(PathBuf, String)> {
    ui_files()
        .into_iter()
        .filter(|(path, _)| path.extension().is_some_and(|ext| ext == "css"))
        .collect()
}

fn js_files() -> Vec<(PathBuf, String)> {
    ui_files()
        .into_iter()
        .filter(|(path, _)| path.extension().is_some_and(|ext| ext == "js"))
        .collect()
}

#[test]
fn tauri_config_matches_the_security_decisions() {
    let config: Value = serde_json::from_str(&read("tauri.conf.json")).expect("valid config");
    assert_eq!(config["app"]["security"]["csp"].as_str(), Some(CSP));
    assert_eq!(config["build"]["frontendDist"].as_str(), Some("ui"));
    assert_eq!(config["bundle"]["active"].as_bool(), Some(false));
    assert_eq!(config["app"]["withGlobalTauri"].as_bool(), Some(true));
    assert_eq!(config["app"]["windows"][0]["title"].as_str(), Some("Modn"));
}

#[test]
fn ui_assets_have_no_remote_or_inline_content() {
    for (path, text) in ui_files() {
        assert!(
            !text.contains("http://") && !text.contains("https://"),
            "{path:?} must not reference a remote origin"
        );
        assert!(
            !text.to_ascii_lowercase().contains("style="),
            "{path:?} must not use a style attribute"
        );
        assert!(
            !text.to_ascii_lowercase().contains("<style"),
            "{path:?} must not use an inline style element"
        );
        assert!(
            !has_event_handler(&text),
            "{path:?} must not use on* attributes"
        );
        assert_scripts_are_external(&path, &text);
    }
}

fn assert_scripts_are_external(path: &Path, text: &str) {
    let lower = text.to_ascii_lowercase();
    let mut cursor = 0;
    while let Some(start) = lower[cursor..].find("<script") {
        let start = cursor + start;
        let end = lower[start..]
            .find('>')
            .unwrap_or_else(|| panic!("unterminated script tag in {path:?}"));
        let tag = &lower[start..start + end + 1];
        assert!(tag.contains("src="), "{path:?} has an inline script tag");
        let body_start = start + end + 1;
        let close = lower[body_start..]
            .find("</script")
            .unwrap_or_else(|| panic!("unterminated script element in {path:?}"));
        let body = text[body_start..body_start + close].trim();
        assert!(body.is_empty(), "{path:?} has an inline script body");
        cursor = body_start + close + "</script".len();
    }
}

fn has_event_handler(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let mut rest = lower.as_str();
    while let Some(open) = rest.find('<') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('>') else {
            break;
        };
        for token in after[..close].split_whitespace() {
            if let Some(equals) = token.find('=') {
                let name = token[..equals].trim_end_matches('/');
                let letters = name.strip_prefix("on").unwrap_or_default();
                if !letters.is_empty() && letters.bytes().all(|byte| byte.is_ascii_lowercase()) {
                    return true;
                }
            }
        }
        rest = &after[close + 1..];
    }
    false
}

#[test]
fn css_variables_are_defined_in_tokens() {
    let tokens = read("ui/css/tokens.css");
    let mut definitions = Vec::new();
    for line in tokens.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("--modn-") {
            if let Some((name, _)) = rest.split_once(':') {
                definitions.push(format!("--modn-{name}"));
            }
        }
    }
    assert!(!definitions.is_empty(), "tokens.css defines no variables");
    for (path, text) in css_files() {
        for usage in var_usages(&text) {
            assert!(
                definitions.iter().any(|definition| definition == &usage),
                "{path:?} uses undefined {usage}"
            );
        }
    }
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

#[test]
fn relative_javascript_imports_resolve() {
    for (path, text) in js_files() {
        let directory = path.parent().expect("javascript file has a directory");
        for specifier in relative_imports(&text) {
            let target = directory.join(&specifier);
            assert!(target.is_file(), "{path:?} imports missing {specifier}");
        }
    }
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
        if let Some(relative) = specifier.strip_prefix("./") {
            imports.push(relative.to_string());
        }
        rest = &after[end..];
    }
    imports
}

#[test]
fn capability_grants_no_plugin_permissions() {
    let capability: Value =
        serde_json::from_str(&read("capabilities/default.json")).expect("valid capability");
    let permissions = capability["permissions"]
        .as_array()
        .expect("permissions array");
    for permission in permissions {
        let permission = permission.as_str().expect("permission string");
        let lower = permission.to_ascii_lowercase();
        for prefix in ["dialog:", "fs:", "shell:", "http:"] {
            assert!(
                !lower.starts_with(prefix),
                "capability must not grant {permission}"
            );
        }
        assert!(
            permission == "core:default" || permission.starts_with("allow-"),
            "capability grants unexpected {permission}"
        );
    }
    let windows = capability["windows"].as_array().expect("windows array");
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].as_str(), Some("main"));
}

#[test]
fn manifest_names_no_remote_plugins() {
    let manifest = read("Cargo.toml");
    for forbidden in ["tauri-plugin-fs", "tauri-plugin-shell", "tauri-plugin-http"] {
        assert!(!manifest.contains(forbidden), "manifest names {forbidden}");
    }
}
