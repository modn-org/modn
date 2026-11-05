use modn_core::game_modules::check_module;
use serde_json::json;
use std::fs;
use tempfile::TempDir;

fn module(minimum: Option<&str>) -> TempDir {
    let directory = tempfile::tempdir().expect("create module fixture");
    let mut manifest = json!({"game_id":"fixture-game", "version":"1.0.0", "api_version":1, "entrypoint":"main.lua"});
    if let Some(minimum) = minimum {
        manifest["min_modn_version"] = json!(minimum);
    }
    fs::write(directory.path().join("module.json"), manifest.to_string())
        .expect("valid test fixture");
    fs::write(
        directory.path().join("main.lua"),
        "return {game_id='fixture-game', api_version=mods_host.api_version}",
    )
    .expect("valid test fixture");
    directory
}

#[test]
fn minimum_at_or_below_client_and_absent_are_accepted() {
    for minimum in [None, Some("0.0.0"), Some(env!("CARGO_PKG_VERSION"))] {
        let directory = module(minimum);
        check_module(directory.path()).expect("compatible module loads");
    }
}

#[test]
fn future_minimum_names_required_and_running_versions_before_lua_runs() {
    let directory = module(Some("999999.0.0"));
    fs::write(directory.path().join("main.lua"), "error('must not run')")
        .expect("valid test fixture");
    let error = format!("{:#}", check_module(directory.path()).unwrap_err());
    assert_eq!(
        error,
        format!(
            "module fixture-game needs Modn 999999.0.0, you have {}",
            env!("CARGO_PKG_VERSION")
        )
    );
}

#[test]
fn malformed_minimum_is_an_input_error() {
    for minimum in ["", "0.3", ">=0.3.0", "banana", "18446744073709551616.0.0"] {
        let directory = module(Some(minimum));
        let error = format!("{:#}", check_module(directory.path()).unwrap_err());
        assert!(error.contains("invalid min_modn_version"), "{error}");
        assert!(error.contains("semantic version"), "{error}");
    }
}

#[test]
fn semantic_version_precedence_ignores_build_metadata() {
    let minimum = format!("{}+module", env!("CARGO_PKG_VERSION"));
    let directory = module(Some(&minimum));
    check_module(directory.path()).expect("build metadata does not change precedence");
    let directory = module(Some("0.2.0-alpha.1"));
    check_module(directory.path()).expect("release accepts earlier prerelease");
}

#[test]
fn cyberpunk_requiring_release_030_is_refused_by_older_development_build() {
    let directory = tempfile::tempdir().expect("valid test fixture");
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../buckets/main/games/cyberpunk-2077");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(source.join("module.json")).expect("valid test fixture"))
            .expect("valid test fixture");
    manifest["min_modn_version"] = json!("0.3.0");
    fs::write(directory.path().join("module.json"), manifest.to_string())
        .expect("valid test fixture");
    fs::copy(
        source.join("module.lua"),
        directory.path().join("module.lua"),
    )
    .expect("valid test fixture");
    let result = check_module(directory.path());
    if semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("valid test fixture")
        < semver::Version::new(0, 3, 0)
    {
        assert_eq!(
            format!("{:#}", result.unwrap_err()),
            format!(
                "module cyberpunk-2077 needs Modn 0.3.0, you have {}",
                env!("CARGO_PKG_VERSION")
            )
        );
    } else {
        result.expect("release can load Cyberpunk module");
    }
}
