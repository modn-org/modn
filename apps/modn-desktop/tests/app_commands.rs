//! Desktop command contracts that span the Rust handlers and the webview API.

use std::fs;
use std::path::Path;

fn read(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"))
}

fn handler_body<'a>(source: &'a str, name: &str) -> &'a str {
    let marker = format!("pub(crate) async fn {name}(");
    let start = source
        .find(&marker)
        .unwrap_or_else(|| panic!("{name} is a command handler"));
    let rest = &source[start + marker.len()..];
    let end = rest.find("\n#[tauri::command]").unwrap_or(rest.len());
    &rest[..end]
}

#[test]
fn install_archive_rechecks_the_plan_and_digest_before_the_write() {
    let source = read("src/app/commands/operations.rs");
    let body = handler_body(&source, "install_archive");
    let recheck = body
        .find("ensure_review_unchanged(")
        .expect("install_archive must recheck the review");
    let write = body
        .find("run_write(&app, &state, run, commit)")
        .expect("install_archive must write through run_write");
    assert!(recheck < write, "the recheck must run before the write");
    assert_eq!(
        body.matches("run_review(").count(),
        2,
        "the review must run again before the write"
    );
}

#[test]
fn install_archive_offers_an_executable_approval() {
    let source = read("src/app/commands/operations.rs");
    let body = handler_body(&source, "install_archive");
    assert!(
        body.contains("executable_approval("),
        "an executable blocker must become an approval ask"
    );
    assert!(
        body.contains("CommandError::approval_required(false, candidates)"),
        "the ask must carry the exact executable paths"
    );
}

#[test]
fn undo_passes_the_approvals_to_the_cli() {
    let source = read("src/app/commands/history.rs");
    let body = handler_body(&source, "undo");
    assert!(body.contains("allow_native_injector"));
    assert!(body.contains("allow_executables"));
    assert!(
        body.contains("args::undo("),
        "undo must validate through args::undo"
    );
}

#[test]
fn the_webview_undo_wrapper_sends_approvals_and_documents_the_error() {
    let source = read("ui/js/api.js");
    let start = source
        .find("export async function undo(")
        .expect("undo wrapper");
    let end = source[start..]
        .find("\n}")
        .map(|offset| start + offset)
        .expect("undo wrapper ends");
    let wrapper = &source[start..end];
    assert!(wrapper.contains("allowNativeInjector"));
    assert!(wrapper.contains("allowExecutables"));
    assert!(
        source.contains("@property {ApprovalRequest} [approval]"),
        "the error's approval field must be documented"
    );
}
