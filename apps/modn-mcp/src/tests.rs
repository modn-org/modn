use crate::config::{identifier, Config, Options};
use crate::progress::Reporter;
use crate::protocol::{Action, Session};
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

async fn reply(session: &mut Session, config: &Config, bytes: &[u8]) -> Value {
    match session.handle(config, bytes).await {
        Action::Reply(value) => value,
        _ => panic!("expected a reply"),
    }
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
fn cancellation_notifications_name_their_target_request() {
    let cancel =
        br#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"a"}}"#;
    assert_eq!(crate::protocol::cancelled_id(cancel), Some(json!("a")));
    assert_eq!(
        crate::protocol::cancelled_id(
            br#"{"jsonrpc":"2.0","id":"a","method":"notifications/cancelled","params":{"requestId":"a"}}"#
        ),
        None
    );
    assert_eq!(
        crate::protocol::cancelled_id(
            br#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{}}"#
        ),
        None
    );
    assert_eq!(
        crate::protocol::request_id(br#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#),
        Some(json!(7))
    );
    assert_eq!(
        crate::protocol::request_id(br#"{"jsonrpc":"2.0","method":"ping"}"#),
        None
    );
}

#[test]
fn queued_cancelled_requests_are_removed() {
    let mut pending = std::collections::VecDeque::from([
        crate::Input::Line(br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#.to_vec()),
        crate::Input::Line(br#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#.to_vec()),
    ]);
    assert!(crate::remove_pending(&mut pending, &json!(1)));
    assert_eq!(pending.len(), 1);
    assert!(!crate::remove_pending(&mut pending, &json!(1)));
    assert_eq!(pending.len(), 1);
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
        let response = reply(&mut session, &config, input).await;
        assert_eq!(response["error"]["code"], code);
        assert_eq!(response["id"], Value::Null);
    }
    let ping = reply(
        &mut session,
        &config,
        br#"{"jsonrpc":"2.0","id":"same","method":"ping"}"#,
    )
    .await;
    assert_eq!(ping["id"], "same");
    assert_eq!(ping["result"], json!({}));
    assert!(matches!(
        session
            .handle(
                &config,
                br#"{"jsonrpc":"2.0","method":"tools/call","params":{"name":"install_archive"}}"#
            )
            .await,
        Action::Silent
    ));
    let early = reply(
        &mut session,
        &config,
        br#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    )
    .await;
    assert_eq!(early["error"]["code"], -32600);
}

fn events(reporter: &mut Reporter, lines: &[&str]) -> Vec<Value> {
    lines
        .iter()
        .filter_map(|line| reporter.event(line))
        .collect()
}

#[test]
fn progress_notifications_follow_each_phase() {
    let mut reporter = Reporter::new(Some(json!("token")));
    let notifications = events(
        &mut reporter,
        &[
            r#"{"phase":"resolve"}"#,
            r#"{"phase":"download","package":"main/game/mod","bytes":0,"total":4096}"#,
            r#"{"phase":"download","package":"main/game/mod","bytes":4096,"total":4096}"#,
            r#"{"phase":"extract","package":"main/game/mod"}"#,
            r#"{"phase":"stage","package":"main/game/mod"}"#,
            r#"{"phase":"commit","package":"main/game/mod"}"#,
            r#"{"phase":"result","outcome":"succeeded"}"#,
        ],
    );
    assert_eq!(notifications.len(), 7);
    assert_eq!(notifications[0]["params"]["message"], json!("resolving"));
    assert_eq!(notifications[1]["params"]["total"], json!(4096));
    assert_eq!(
        notifications[1]["params"]["message"],
        json!("downloading main/game/mod")
    );
    assert_eq!(
        notifications[3]["params"]["message"],
        json!("extracting main/game/mod")
    );
    assert_eq!(
        notifications[5]["params"]["message"],
        json!("committing main/game/mod")
    );
    assert_eq!(notifications[6]["params"]["message"], json!("succeeded"));
    let mut last = 0;
    for notification in &notifications {
        assert_eq!(notification["params"]["progressToken"], json!("token"));
        let progress = notification["params"]["progress"]
            .as_u64()
            .expect("progress");
        assert!(progress > last, "progress must increase");
        last = progress;
    }
    assert!(reporter.is_committed());
    assert!(!reporter.is_cancelled());
}

#[test]
fn no_token_reports_nothing_but_tracks_the_commit_point() {
    let mut reporter = Reporter::new(None);
    assert!(events(
        &mut reporter,
        &[
            r#"{"phase":"resolve"}"#,
            r#"{"phase":"commit","package":null}"#,
            r#"{"phase":"result","outcome":"failed"}"#,
        ],
    )
    .is_empty());
    assert!(reporter.is_committed());
    assert!(!reporter.is_cancelled());
}

#[test]
fn cancellation_before_the_commit_point_stops_the_call() {
    let completion = crate::backend::Completion {
        result: Err(anyhow::anyhow!("backend failed with exit 1")),
        committed: false,
        cancelled: true,
    };
    assert!(matches!(tools::reply(completion), tools::Reply::Cancelled));
}

#[test]
fn cancellation_after_the_commit_point_completes_the_call() {
    let mut reporter = Reporter::new(None);
    reporter.event(r#"{"phase":"commit","package":"main/game/mod"}"#);
    assert!(reporter.is_committed());
    let completion = crate::backend::Completion {
        result: Ok(json!({"installed":true})),
        committed: true,
        cancelled: false,
    };
    match tools::reply(completion) {
        tools::Reply::Response(response) => assert_eq!(response["isError"], false),
        tools::Reply::Cancelled => panic!("a committed call must respond"),
    }
}

#[test]
fn malformed_progress_lines_are_ignored() {
    let mut reporter = Reporter::new(Some(json!(7)));
    assert!(reporter.event("not JSON").is_none());
    assert!(reporter.event(r#"{"phase":"unknown"}"#).is_none());
    assert!(!reporter.is_committed());
}
