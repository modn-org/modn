use crate::config::{identifier, Config, Options};
use crate::protocol::Session;
use crate::tools;
use clap::Parser;
use serde_json::{json, Value};
use tempfile::TempDir;

fn fixture() -> (TempDir, Config) {
    let temporary = tempfile::tempdir().expect("temporary fixture");
    let state = temporary.path().join("modn");
    std::fs::create_dir(&state).expect("state");
    let options = Options::parse_from([
        "modn-mcp".into(),
        "--backend".into(),
        std::env::current_exe()
            .expect("executable")
            .into_os_string(),
        "--state-dir".into(),
        state.into_os_string(),
        "--allow-root".into(),
        temporary.path().as_os_str().to_owned(),
    ]);
    let config = Config::new(options).expect("valid configuration");
    (temporary, config)
}

#[test]
fn identifiers_refuse_switches_and_traversal() {
    for value in [
        "--yes",
        "../outside",
        "/absolute",
        "a\\b",
        "a\0b",
        "",
        "a b",
    ] {
        assert!(identifier(value).is_err(), "accepted {value:?}");
    }
    assert!(identifier("main/cyberpunk-2077/cet").is_ok());
}

#[test]
fn paths_require_scope_and_separate_state() {
    let (temporary, config) = fixture();
    let archive = temporary.path().join("a.zip");
    std::fs::write(&archive, b"fixture").expect("archive");
    assert!(config
        .input_path(archive.to_str().expect("UTF-8"), false)
        .is_ok());
    assert!(config.input_path("a.zip", false).is_err());
    assert!(config
        .input_path(config.state.to_str().expect("UTF-8"), true)
        .is_err());
    assert!(config
        .input_path(
            temporary
                .path()
                .parent()
                .expect("parent")
                .to_str()
                .expect("UTF-8"),
            true
        )
        .is_err());
}

#[test]
fn previews_and_writes_have_distinct_approval_contracts() {
    let (temporary, config) = fixture();
    let archive = temporary.path().join("a.zip");
    let game = temporary.path().join("game");
    std::fs::write(&archive, b"fixture").expect("archive");
    std::fs::create_dir(&game).expect("game");
    let input = json!({"archive":archive,"game_path":game,"game_id":"sample-game"});
    let args = tools::arguments(&config, "preview_install", input.clone()).expect("preview");
    assert!(args.contains(&"--dry-run".into()));
    assert!(!args.contains(&"--download".into()));
    assert!(tools::arguments(&config, "install_archive", input.clone()).is_err());
    let mut approved = input;
    approved["approved"] = json!(true);
    assert!(tools::arguments(&config, "install_archive", approved.clone()).is_err());
    assert!(tools::arguments(&config, "preview_install", approved).is_err());
}

#[test]
fn arbitrary_backend_options_are_not_arguments() {
    let (_temporary, config) = fixture();
    for input in [
        json!({"argv":["recover","--yes"]}),
        json!({"state_dir":"other"}),
        json!([]),
        json!(null),
        json!(true),
        json!(1),
        json!("arguments"),
    ] {
        assert!(tools::arguments(&config, "registered_games", input).is_err());
    }
    assert!(tools::arguments(&config, "catalog_search", json!({"query":"--yes"})).is_err());
    assert_eq!(
        tools::list(false)["tools"].as_array().expect("tools").len(),
        5
    );
    assert!(!tools::known("install_archive", false));
}

#[tokio::test]
async fn protocol_ids_lifecycle_and_errors() {
    let (_temporary, config) = fixture();
    let mut session = Session::default();
    for (input, code) in [
        (b"{".as_slice(), -32700),
        (b"[]".as_slice(), -32600),
        (
            br#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#.as_slice(),
            -32600,
        ),
    ] {
        let response = session
            .handle(&config, input)
            .await
            .expect("error response");
        assert_eq!(response["error"]["code"], code);
        assert_eq!(response["id"], Value::Null);
    }
    let ping = session
        .handle(&config, br#"{"jsonrpc":"2.0","id":"same","method":"ping"}"#)
        .await
        .expect("ping response");
    assert_eq!(ping["id"], "same");
    assert_eq!(ping["result"], json!({}));
    assert!(session
        .handle(
            &config,
            br#"{"jsonrpc":"2.0","method":"tools/call","params":{"name":"install_archive"}}"#
        )
        .await
        .is_none());
    let early = session
        .handle(
            &config,
            br#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        )
        .await
        .expect("early error");
    assert_eq!(early["error"]["code"], -32600);
}
