//! Contract tests: the registered handlers, the capability, the build manifest
//! and the webview API must name the same commands.

use super::{COMMANDS, progress_payload};
use crate::backend::ProgressEvent;
use serde_json::Value;
use std::fs;
use std::path::Path;

fn read(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"))
}

fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names.dedup();
    names
}

fn contract() -> Vec<String> {
    sorted(COMMANDS.iter().map(|name| (*name).to_string()).collect())
}

#[test]
fn command_contract_has_no_duplicates() {
    assert_eq!(contract().len(), COMMANDS.len());
}

#[test]
fn capability_lists_exactly_the_registered_commands() {
    let capability: Value =
        serde_json::from_str(&read("capabilities/default.json")).expect("valid capability");
    let permissions = capability["permissions"]
        .as_array()
        .expect("permissions array");
    let mut allowed = Vec::new();
    let mut core = 0;
    for permission in permissions {
        let permission = permission.as_str().expect("permission string");
        let lower = permission.to_ascii_lowercase();
        for prefix in ["dialog:", "fs:", "shell:", "http:"] {
            assert!(
                !lower.starts_with(prefix),
                "capability must not grant {permission}"
            );
        }
        if permission == "core:default" {
            core += 1;
            continue;
        }
        let name = permission
            .strip_prefix("allow-")
            .unwrap_or_else(|| panic!("capability grants unexpected {permission}"));
        allowed.push(name.replace('-', "_"));
    }
    assert_eq!(core, 1, "capability must grant core:default once");
    assert_eq!(sorted(allowed), contract());
}

#[test]
fn app_registers_exactly_the_command_contract() {
    let source = read("src/app.rs");
    let marker = "generate_handler![";
    let start = source.find(marker).expect("app registers handlers") + marker.len();
    let end = source[start..]
        .find(']')
        .map(|offset| start + offset)
        .expect("handler list ends");
    let registered = source[start..end]
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            entry
                .rsplit("::")
                .next()
                .expect("a command path has a name")
                .to_string()
        })
        .collect();
    assert_eq!(sorted(registered), contract());
}

#[test]
fn build_manifest_lists_exactly_the_command_contract() {
    let source = read("build.rs");
    let marker = "const COMMANDS: &[&str] = &[";
    let start = source.find(marker).expect("build manifest lists commands") + marker.len();
    let end = source[start..]
        .find("];")
        .map(|offset| start + offset)
        .expect("manifest list ends");
    let listed = source[start..end]
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect();
    assert_eq!(sorted(listed), contract());
}

#[test]
fn webview_api_invokes_exactly_the_registered_commands() {
    let source = read("ui/js/api.js");
    let mut invoked = Vec::new();
    let mut rest = source.as_str();
    while let Some(index) = rest.find("invoke('") {
        let after = &rest[index + "invoke('".len()..];
        let Some(end) = after.find('\'') else {
            break;
        };
        invoked.push(after[..end].to_string());
        rest = &after[end..];
    }
    assert_eq!(sorted(invoked), contract());
}

#[test]
fn progress_payload_uses_the_contract_shape() {
    let event = ProgressEvent {
        phase: "commit".into(),
        package: Some("main/example/mod".into()),
        bytes: None,
        total: None,
        outcome: None,
        error: None,
    };
    let payload = progress_payload("run-1", &event);
    assert_eq!(payload["runId"], "run-1");
    assert_eq!(payload["event"]["phase"], "commit");
    assert_eq!(payload["event"]["package"], "main/example/mod");
    assert!(payload["event"].get("bytes").is_none());
    assert!(payload["event"].get("outcome").is_none());
}
