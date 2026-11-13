//! Contract tests: the registered handlers, the capability, the build manifest
//! and the webview API must name the same commands.

use super::{COMMANDS, ensure_not_cancelled, progress_payload};
use crate::app::state::Runs;
use crate::backend::ProgressEvent;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::sync::Arc;

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

#[test]
fn a_cancel_that_arrives_during_the_checks_refuses_the_write() {
    let runs = Arc::new(Runs::default());
    let run = runs.start("run-1");
    ensure_not_cancelled(&run).expect("a fresh run may write");

    // The archive review or catalog lookup runs while the guard is held.
    assert!(runs.cancel("run-1"));

    let error = ensure_not_cancelled(&run).expect_err("a cancelled run must not write");
    assert_eq!(error.code, "CANCELLED");
}

fn handler_body<'a>(source: &'a str, name: &str) -> &'a str {
    let marker = format!("pub(crate) async fn {name}(");
    let start = source
        .find(&marker)
        .unwrap_or_else(|| panic!("{name} is a write command"));
    let rest = &source[start + marker.len()..];
    let end = rest.find("\n#[tauri::command]").unwrap_or(rest.len());
    &rest[..end]
}

#[test]
fn every_write_command_registers_cancellation_before_its_checks() {
    for (file, handlers) in [
        (
            "src/app/commands/operations.rs",
            &["install_archive", "install_mod", "uninstall", "update"][..],
        ),
        ("src/app/commands/history.rs", &["undo"][..]),
    ] {
        let source = read(file);
        for name in handlers {
            let body = handler_body(&source, name);
            let registered = body
                .find("state.runs.start(&run_id)")
                .unwrap_or_else(|| panic!("{name} must register its run first"));
            for marker in ["run_blocking(", "args::", "run_write("] {
                if let Some(index) = body.find(marker) {
                    assert!(
                        registered < index,
                        "{name} must register the run before {marker}"
                    );
                }
            }
            assert!(
                body.contains("run_write(&app, &state, run, commit)"),
                "{name} must pass its registered run to the write"
            );
        }
    }
}
