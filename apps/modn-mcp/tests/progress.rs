use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

fn fixture_backend() -> PathBuf {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/backend.rs");
        let name = if cfg!(windows) {
            "modn-mcp-progress-fixture.exe"
        } else {
            "modn-mcp-progress-fixture"
        };
        let output = std::env::temp_dir().join(format!("{}-{name}", std::process::id()));
        let compiled = Command::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&output)
            .output()
            .expect("rustc runs");
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        output
    })
    .clone()
}

struct Server {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
}

impl Server {
    fn start(root: &Path) -> Self {
        Self::start_mode(root, "progress", 5)
    }

    fn start_mode(root: &Path, mode: &str, timeout_seconds: u64) -> Self {
        let state = root.join("modn");
        std::fs::create_dir(&state).expect("state");
        std::fs::write(state.join("mode"), mode).expect("mode");
        let mut child = Command::new(env!("CARGO_BIN_EXE_modn-mcp"))
            .arg("--backend")
            .arg(fixture_backend())
            .arg("--state-dir")
            .arg(&state)
            .arg("--allow-root")
            .arg(root)
            .arg("--allow-writes")
            .arg("--timeout-seconds")
            .arg(timeout_seconds.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("server starts");
        let stdin = child.stdin.take().expect("server stdin");
        let stdout = BufReader::new(child.stdout.take().expect("server stdout"));
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in stdout.lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            stdin,
            lines,
        }
    }

    fn send(&mut self, value: Value) {
        writeln!(self.stdin, "{value}").expect("message write");
        self.stdin.flush().expect("message flush");
    }

    fn send_raw(&mut self, text: &str) {
        self.stdin.write_all(text.as_bytes()).expect("raw write");
        self.stdin.write_all(b"\n").expect("raw newline");
        self.stdin.flush().expect("raw flush");
    }

    fn next(&mut self) -> Value {
        let line = self
            .lines
            .recv_timeout(Duration::from_secs(15))
            .expect("server message");
        serde_json::from_str(&line).expect("server JSON")
    }

    fn initialize(&mut self) {
        self.send(
            json!({"jsonrpc":"2.0","id":"init","method":"initialize","params":{
            "protocolVersion":"2025-06-18","capabilities":{},
            "clientInfo":{"name":"progress-test","version":"1"}}}),
        );
        let response = self.next();
        assert_eq!(response["result"]["protocolVersion"], "2025-06-18");
        self.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    }

    fn send_tool(&mut self, id: Value, name: &str, arguments: Value, token: Option<Value>) {
        let mut params = json!({"name":name,"arguments":arguments});
        if let Some(token) = token {
            params["_meta"] = json!({"progressToken": token});
        }
        self.send(json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":params}));
    }

    fn call(&mut self, id: Value, token: Option<Value>) -> (Value, Vec<Value>) {
        self.send_tool(id, "registered_games", json!({}), token);
        let mut notifications = Vec::new();
        loop {
            let message = self.next();
            if message.get("method").is_some() {
                notifications.push(message);
            } else {
                return (message, notifications);
            }
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn progress_notifications_increase_with_message_and_total() {
    let temporary = tempfile::tempdir().expect("fixture");
    let mut server = Server::start(temporary.path());
    server.initialize();
    let (response, notifications) = server.call(json!(1), Some(json!("progress-token")));
    assert_eq!(response["id"], json!(1), "{response}");
    assert_eq!(response["result"]["isError"], false);
    assert!(notifications.len() >= 6, "one notification per phase");
    let mut last = 0;
    for notification in &notifications {
        assert_eq!(notification["method"], "notifications/progress");
        assert_eq!(
            notification["params"]["progressToken"],
            json!("progress-token")
        );
        let progress = notification["params"]["progress"]
            .as_u64()
            .expect("progress value");
        assert!(progress > last, "progress must increase");
        last = progress;
        assert!(notification["params"]["message"].is_string());
    }
    assert_eq!(notifications[0]["params"]["message"], json!("resolving"));
    assert!(notifications
        .iter()
        .any(|notification| notification["params"]["total"] == json!(64)));
    assert_eq!(
        notifications.last().expect("result")["params"]["message"],
        json!("succeeded")
    );
}

#[test]
fn cancellation_before_commit_sends_no_response_and_keeps_serving() {
    let temporary = tempfile::tempdir().expect("fixture");
    let mut server = Server::start(temporary.path());
    server.initialize();
    server.send(
        json!({"jsonrpc":"2.0","id":10,"method":"tools/call","params":{
        "name":"registered_games","arguments":{},"_meta":{"progressToken":"cancel-before"}}}),
    );
    let first = server.next();
    assert_eq!(first["method"], "notifications/progress");
    server.send(
        json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{
        "requestId":10,"reason":"test"}}),
    );
    server.send(json!({"jsonrpc":"2.0","id":11,"method":"ping"}));
    loop {
        let message = server.next();
        if message.get("method").is_some() {
            continue;
        }
        assert_eq!(
            message["id"],
            json!(11),
            "a cancelled call must not respond"
        );
        assert_eq!(message["result"], json!({}));
        break;
    }
}

#[test]
fn cancellation_after_commit_is_ignored_and_the_call_completes() {
    let temporary = tempfile::tempdir().expect("fixture");
    let mut server = Server::start(temporary.path());
    server.initialize();
    server.send(
        json!({"jsonrpc":"2.0","id":20,"method":"tools/call","params":{
        "name":"registered_games","arguments":{},"_meta":{"progressToken":"cancel-after"}}}),
    );
    loop {
        let message = server.next();
        assert!(
            message.get("method").is_some(),
            "the call responded before its commit point"
        );
        if message["params"]["message"]
            .as_str()
            .is_some_and(|text| text.starts_with("committing"))
        {
            break;
        }
    }
    server.send(
        json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{
        "requestId":20,"reason":"test"}}),
    );
    loop {
        let message = server.next();
        if message.get("method").is_some() {
            continue;
        }
        assert_eq!(message["id"], json!(20), "{message}");
        assert_eq!(message["result"]["isError"], false);
        break;
    }
}

#[test]
fn a_call_without_a_token_is_unchanged() {
    let temporary = tempfile::tempdir().expect("fixture");
    let mut server = Server::start(temporary.path());
    server.initialize();
    let (response, notifications) = server.call(json!(30), None);
    assert!(notifications.is_empty());
    assert_eq!(response["id"], json!(30), "{response}");
    assert_eq!(response["result"]["isError"], false);
}

#[test]
fn a_full_queue_still_reads_cancellation_for_the_active_call() {
    let temporary = tempfile::tempdir().expect("fixture");
    let mut server = Server::start_mode(temporary.path(), "queued", 5);
    server.initialize();
    server.send_tool(
        json!("active"),
        "registered_games",
        json!({}),
        Some(json!("active-token")),
    );
    let first = server.next();
    assert_eq!(first["method"], "notifications/progress", "{first}");
    for id in [100, 101, 102, 103] {
        server.send(json!({"jsonrpc":"2.0","id":id,"method":"ping"}));
    }
    server.send(
        json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"active"}}),
    );
    let mut answered = 0;
    let mut active_answered = false;
    while answered < 4 {
        let message = server.next();
        if message.get("method").is_some() {
            continue;
        }
        match message["id"].as_i64() {
            Some(id) if (100..=103).contains(&id) => answered += 1,
            _ => active_answered = true,
        }
    }
    assert!(!active_answered, "a cancelled call must not respond");
    assert!(
        !temporary.path().join("modn").join("commit").exists(),
        "the CLI must stop before its commit point"
    );
}

#[test]
fn a_full_queue_still_answers_malformed_and_oversized_input() {
    let temporary = tempfile::tempdir().expect("fixture");
    let mut server = Server::start_mode(temporary.path(), "queued", 5);
    server.initialize();
    server.send_tool(
        json!("active"),
        "registered_games",
        json!({}),
        Some(json!("active-token")),
    );
    let first = server.next();
    assert_eq!(first["method"], "notifications/progress", "{first}");
    for id in [110, 111, 112, 113] {
        server.send(json!({"jsonrpc":"2.0","id":id,"method":"ping"}));
    }
    server.send_raw("{");
    server.send_raw(&"x".repeat(64 * 1024 + 1));
    server.send(
        json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"active"}}),
    );
    let mut parse_error = false;
    let mut oversize_error = false;
    let mut answered = 0;
    let mut active_answered = false;
    while !(parse_error && oversize_error && answered == 4) {
        let message = server.next();
        if message.get("method").is_some() {
            continue;
        }
        if message["id"].is_null() {
            match message["error"]["code"].as_i64() {
                Some(-32700) => parse_error = true,
                Some(-32600) => oversize_error = true,
                _ => {}
            }
            continue;
        }
        match message["id"].as_i64() {
            Some(id) if (110..=113).contains(&id) => answered += 1,
            _ => active_answered = true,
        }
    }
    assert!(!active_answered, "a cancelled call must not respond");
    assert!(
        !temporary.path().join("modn").join("commit").exists(),
        "the CLI must stop before its commit point"
    );
}

#[test]
fn a_cancelled_queued_request_never_runs_or_responds() {
    let temporary = tempfile::tempdir().expect("fixture");
    let mut server = Server::start_mode(temporary.path(), "queued", 5);
    server.initialize();
    server.send_tool(
        json!("active"),
        "registered_games",
        json!({}),
        Some(json!("active-token")),
    );
    let first = server.next();
    assert_eq!(first["method"], "notifications/progress", "{first}");
    server.send_tool(
        json!("queued"),
        "catalog_search",
        json!({"query":"queued-b"}),
        None,
    );
    server.send(
        json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"queued"}}),
    );
    server.send(json!({"jsonrpc":"2.0","id":"probe","method":"ping"}));
    let mut queued_answered = false;
    loop {
        let message = server.next();
        if message.get("method").is_some() {
            continue;
        }
        if message["id"] == json!("probe") {
            break;
        }
        if message["id"] == json!("queued") {
            queued_answered = true;
        }
    }
    assert!(
        !queued_answered,
        "a cancelled queued request must not respond"
    );
    let state = temporary.path().join("modn");
    let invocations = std::fs::read_to_string(state.join("invocations")).expect("invocations");
    assert_eq!(invocations.lines().count(), 1, "{invocations}");
    assert!(!invocations.contains("queued-b"), "{invocations}");
}

#[test]
fn the_final_progress_event_precedes_the_response() {
    let temporary = tempfile::tempdir().expect("fixture");
    let mut server = Server::start_mode(temporary.path(), "burst", 5);
    server.initialize();
    let (response, notifications) = server.call(json!(40), Some(json!("burst-token")));
    assert_eq!(response["id"], json!(40), "{response}");
    assert_eq!(response["result"]["isError"], false, "{response}");
    assert_eq!(notifications.len(), 34, "{notifications:?}");
    assert_eq!(
        notifications.last().expect("result")["params"]["message"],
        json!("succeeded")
    );
}

#[test]
fn progress_lines_do_not_count_toward_the_stderr_limit() {
    let temporary = tempfile::tempdir().expect("fixture");
    let mut server = Server::start_mode(temporary.path(), "many", 5);
    server.initialize();
    let (response, _) = server.call(json!(50), None);
    assert_eq!(response["id"], json!(50), "{response}");
    assert_eq!(response["result"]["isError"], false, "{response}");
}

#[test]
fn a_split_progress_line_survives_a_competing_select_branch() {
    let temporary = tempfile::tempdir().expect("fixture");
    let mut server = Server::start_mode(temporary.path(), "split", 5);
    server.initialize();
    server.send_tool(
        json!(60),
        "registered_games",
        json!({}),
        Some(json!("split-token")),
    );
    let first = server.next();
    assert_eq!(first["method"], "notifications/progress", "{first}");
    std::thread::sleep(Duration::from_millis(200));
    server.send(
        json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":60}}),
    );
    server.send(json!({"jsonrpc":"2.0","id":"probe","method":"ping"}));
    loop {
        let message = server.next();
        if message.get("method").is_some() {
            continue;
        }
        assert_eq!(
            message["id"],
            json!(60),
            "the call did not observe the split commit event"
        );
        break;
    }
}

#[test]
fn stdout_over_the_limit_fails_without_waiting_for_the_child() {
    let temporary = tempfile::tempdir().expect("fixture");
    let mut server = Server::start_mode(temporary.path(), "huge", 5);
    server.initialize();
    let started = Instant::now();
    let (response, _) = server.call(json!(70), None);
    let elapsed = started.elapsed();
    assert_eq!(response["id"], json!(70), "{response}");
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("error text");
    assert!(text.contains("output exceeds"), "{response}");
    assert!(elapsed < Duration::from_secs(4), "took {elapsed:?}");
}
