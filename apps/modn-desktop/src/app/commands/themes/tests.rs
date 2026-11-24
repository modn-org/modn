//! Theme command contracts: request vectors, resolve post-processing and the
//! import plan, without a native dialog or a Tauri window.

use super::{
    ImportPlan, cancelled_answer, default_theme_answer, import_plan, resolve_request,
    resolved_theme, select_request, stale_answer,
};
use crate::app::theme_assets::ASSET_BASE;
use serde_json::{Value, json};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

fn strings(args: Vec<OsString>) -> Vec<String> {
    args.into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

fn unique_folder() -> PathBuf {
    static SEQUENCE: AtomicU64 = AtomicU64::new(1);
    let folder = std::env::temp_dir().join(format!(
        "modn-desktop-theme-command-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    // Windows reuses process ids, so an earlier run may have left this folder behind.
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir(&folder).expect("create the disposable theme folder");
    folder
}

#[test]
fn a_theme_select_names_the_pair_and_its_variant() {
    let args = select_request("cyberpunk-2077", Some("night-city"), Some("neon"))
        .expect("a named theme accepts a variant");
    assert_eq!(
        strings(args),
        [
            "theme",
            "select",
            "cyberpunk-2077",
            "night-city",
            "--variant",
            "neon"
        ]
    );
}

#[test]
fn a_missing_theme_selects_the_explicit_default() {
    let args = select_request("cyberpunk-2077", None, None).expect("a valid game");
    assert_eq!(
        strings(args),
        ["theme", "select", "cyberpunk-2077", "--none"]
    );
}

#[test]
fn a_variant_without_a_theme_is_refused() {
    let error = select_request("cyberpunk-2077", None, Some("neon"))
        .expect_err("a variant belongs to a named theme");
    assert_eq!(error.code, "INVALID_ID");
}

#[test]
fn a_select_with_an_invalid_game_or_variant_is_refused() {
    let too_long = "a".repeat(65);
    for (game, theme, variant) in [
        ("Night-City", None, None),
        (&too_long, None, None),
        ("cyberpunk-2077", Some("night-city"), Some("Neon")),
        ("cyberpunk-2077", Some("Night-City"), None),
    ] {
        let error = select_request(game, theme, variant).expect_err("an invalid id is refused");
        assert_eq!(error.code, "INVALID_ID", "{game} must be refused");
    }
}

#[test]
fn the_default_theme_flag_answers_without_a_cli_request() {
    assert!(
        resolve_request(true, "cyberpunk-2077")
            .expect("the flag needs no CLI request")
            .is_none()
    );
}

#[test]
fn a_normal_resolve_request_names_the_game() {
    let args = resolve_request(false, "cyberpunk-2077")
        .expect("valid game")
        .expect("the CLI runs");
    assert_eq!(
        strings(args),
        ["theme", "resolve", "--game", "cyberpunk-2077"]
    );
}

#[test]
fn an_invalid_game_id_is_refused_before_the_flag_decision() {
    let too_long = "a".repeat(65);
    for game in [
        "",
        "Night-City",
        "a b",
        "-night",
        "night-",
        "night--city",
        "night_city",
        &too_long,
    ] {
        let error = resolve_request(true, game).expect_err("an invalid id is refused");
        assert_eq!(error.code, "INVALID_ID", "{game} must be refused");
    }
}

#[test]
fn the_default_theme_answer_has_the_documented_shape() {
    let answer = default_theme_answer("cyberpunk-2077");
    assert_eq!(answer["game"], "cyberpunk-2077");
    assert_eq!(answer["theme"], Value::Null);
    assert_eq!(answer["fallback"], Value::Null);
    assert_eq!(answer["disabled"], "default-theme-flag");
    assert_eq!(answer["diagnostics"], json!([]));
    assert_eq!(answer["asset_base"], ASSET_BASE);
    assert!(answer.get("assets_dir").is_none());
}

#[test]
fn a_null_theme_stores_no_root_and_adds_the_asset_base() {
    let resolved = resolved_theme(json!({
        "game": "cyberpunk-2077",
        "theme": null,
        "fallback": null,
        "diagnostics": [],
    }))
    .expect("the default theme is a valid answer");
    assert!(resolved.root.is_none());
    assert_eq!(resolved.answer["theme"], Value::Null);
    assert_eq!(resolved.answer["asset_base"], ASSET_BASE);
    assert!(resolved.answer.get("assets_dir").is_none());
}

#[test]
fn a_missing_theme_key_stores_no_root() {
    let resolved =
        resolved_theme(json!({"game": "cyberpunk-2077"})).expect("an absent theme is the default");
    assert!(resolved.root.is_none());
    assert_eq!(resolved.answer["asset_base"], ASSET_BASE);
}

#[test]
fn a_theme_answer_strips_assets_dir_and_stores_the_directory() {
    let folder = unique_folder();
    let resolved = resolved_theme(json!({
        "game": "cyberpunk-2077",
        "theme": {
            "id": "night-city",
            "assets_dir": folder.to_string_lossy(),
        },
        "fallback": null,
        "diagnostics": [],
    }))
    .expect("an existing absolute directory is valid");
    assert_eq!(resolved.root.as_deref(), Some(folder.as_path()));
    assert_eq!(resolved.answer["theme"]["id"], "night-city");
    assert!(resolved.answer["theme"].get("assets_dir").is_none());
    assert_eq!(resolved.answer["asset_base"], ASSET_BASE);
    fs::remove_dir_all(&folder).expect("remove the disposable folder");
}

#[test]
fn a_relative_assets_dir_is_refused() {
    let error = resolved_theme(json!({
        "theme": {"assets_dir": "themes/cyberpunk-2077/night-city"},
    }))
    .expect_err("a relative directory cannot be served");
    assert_eq!(error.code, "THEME_INVALID");
}

#[test]
fn a_missing_assets_dir_is_refused() {
    let folder = unique_folder();
    let missing = folder.join("missing");
    let error = resolved_theme(json!({"theme": {"assets_dir": missing.to_string_lossy()}}))
        .expect_err("a missing directory cannot be served");
    assert_eq!(error.code, "THEME_INVALID");
    fs::remove_dir_all(&folder).expect("remove the disposable folder");
}

#[test]
fn a_missing_or_non_string_assets_dir_is_refused() {
    for answer in [
        json!({"theme": {}}),
        json!({"theme": {"assets_dir": null}}),
        json!({"theme": {"assets_dir": 7}}),
    ] {
        let error = resolved_theme(answer).expect_err("a theme needs its directory");
        assert_eq!(error.code, "THEME_INVALID");
    }
}

#[test]
fn a_non_object_theme_is_refused() {
    let error =
        resolved_theme(json!({"theme": "night-city"})).expect_err("a theme is an object or null");
    assert_eq!(error.code, "THEME_INVALID");
}

#[test]
fn a_non_object_answer_is_refused() {
    let error = resolved_theme(json!(["theme"])).expect_err("an answer is an object");
    assert_eq!(error.code, "THEME_INVALID");
}

#[test]
fn a_resolved_answer_keeps_the_fallback_and_diagnostics() {
    let resolved = resolved_theme(json!({
        "game": "cyberpunk-2077",
        "theme": null,
        "fallback": {"reason": "theme-invalid", "message": "theme night-city is invalid"},
        "diagnostics": [{"rule": "unknown-property", "message": "property \"transform\" is not allowed"}],
    }))
    .expect("a fallback answer is valid");
    assert_eq!(resolved.answer["fallback"]["reason"], "theme-invalid");
    assert_eq!(
        resolved.answer["diagnostics"][0]["rule"],
        "unknown-property"
    );
}

#[test]
fn a_superseded_resolve_answers_with_only_stale_and_game() {
    let answer = stale_answer("cyberpunk-2077");
    assert_eq!(answer, json!({"stale": true, "game": "cyberpunk-2077"}));
    assert_eq!(
        answer
            .as_object()
            .expect("the stale answer is an object")
            .len(),
        2
    );
}

#[test]
fn an_import_without_a_folder_cancels_without_a_request() {
    let plan = import_plan(None);
    assert!(matches!(plan, ImportPlan::Cancelled));
    assert_eq!(cancelled_answer(), json!({"cancelled": true}));
}

#[test]
fn an_import_registers_the_chosen_folder() {
    match import_plan(Some(Path::new(r"C:\themes\night-city"))) {
        ImportPlan::Register(args) => assert_eq!(
            strings(args),
            ["theme", "register", r"C:\themes\night-city"]
        ),
        ImportPlan::Cancelled => panic!("a chosen folder must register"),
    }
}
