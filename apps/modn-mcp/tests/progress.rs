use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::OnceLock;
use std::time::Duration;

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
        let state = root.join("modn");
        std::fs::create_dir(&state).expect("state");
        std::fs::write(state.join("mode"), "progress").expect("mode");
        let mut child = Command::new(env!("CARGO_BIN_EXE_modn-mcp"))
            .arg("--backend")
            .arg(fixture_backend())
            .arg("--state-dir")
            .arg(&state)
            .arg("--allow-root")
            .arg(root)
            .arg("--allow-writes")
            .arg("--timeout-seconds")
            .arg("5")
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

    fn call(&mut self, id: Value, token: Option<Value>) -> (Value, Vec<Value>) {
        let mut params = json!({"name":"registered_games","arguments":{}});
        if let Some(token) = token {
            params["_meta"] = json!({"progressToken": token});
        }
        self.send(json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":params}));
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
