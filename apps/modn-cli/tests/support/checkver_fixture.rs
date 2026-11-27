#![allow(
    dead_code,
    reason = "shared fixture exposes helpers for different test crates"
)]

use crate::main_bucket;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

pub struct Request {
    pub target: String,
    pub headers: BTreeMap<String, String>,
}

pub struct MockServer {
    pub address: String,
    requests: Arc<Mutex<Vec<Request>>>,
    shutdown: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl MockServer {
    pub fn new(routes: BTreeMap<String, (u16, Vec<u8>)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("mock listener");
        listener
            .set_nonblocking(true)
            .expect("nonblocking mock listener");
        let address = format!("http://{}", listener.local_addr().expect("mock address"));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let routes = Arc::new(routes);
        let thread = {
            let requests = Arc::clone(&requests);
            let shutdown = Arc::clone(&shutdown);
            thread::spawn(move || {
                while !shutdown.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let routes = Arc::clone(&routes);
                            let requests = Arc::clone(&requests);
                            thread::spawn(move || serve(stream, &routes, &requests));
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            })
        };
        Self {
            address,
            requests,
            shutdown,
            thread: Some(thread),
        }
    }

    pub fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .expect("request log")
            .iter()
            .map(|request| request.target.clone())
            .collect()
    }

    pub fn header(&self, target_contains: &str, name: &str) -> Option<String> {
        self.requests
            .lock()
            .expect("request log")
            .iter()
            .find(|request| request.target.contains(target_contains))
            .and_then(|request| request.headers.get(&name.to_ascii_lowercase()).cloned())
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(
    mut stream: std::net::TcpStream,
    routes: &BTreeMap<String, (u16, Vec<u8>)>,
    requests: &Mutex<Vec<Request>>,
) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 2048];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                buffer.extend_from_slice(&chunk[..read]);
                if buffer.windows(4).any(|window| window == b"\r\n\r\n") || buffer.len() > 64 * 1024
                {
                    break;
                }
            }
        }
    }
    let text = String::from_utf8_lossy(&buffer);
    let mut lines = text.split("\r\n");
    let target = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .unwrap_or("")
        .to_owned();
    let headers: BTreeMap<String, String> = lines
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    requests.lock().expect("request log").push(Request {
        target: target.clone(),
        headers,
    });
    let (status, body) = routes
        .get(&target)
        .cloned()
        .unwrap_or((404, b"not found".to_vec()));
    let reason = if status == 200 { "OK" } else { "Not Found" };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&body);
}

pub struct Fixture {
    pub temp: TempDir,
    pub data: TempDir,
    pub catalog: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().expect("fixture");
        let data = tempfile::tempdir().expect("data directory");
        let catalog = temp.path().join("catalog");
        fs::create_dir_all(catalog.join("mods")).expect("catalog");
        fs::write(
            catalog.join("bucket.json"),
            br#"{"schema_version":2,"name":"fixture","title":"Fixture"}"#,
        )
        .expect("bucket metadata");
        fs::write(catalog.join("catalog.sig"), b"untouched signature bytes")
            .expect("signature fixture");
        Self {
            temp,
            data,
            catalog,
        }
    }

    pub fn write(&self, game: &str, id: &str, manifest: &Value) {
        let directory = self.catalog.join("mods").join(game);
        fs::create_dir_all(&directory).expect("game directory");
        fs::write(
            directory.join(format!("{id}.json")),
            serde_json::to_vec(manifest).expect("manifest JSON"),
        )
        .expect("manifest");
    }

    pub fn write_text(&self, game: &str, id: &str, text: &str) {
        let path = self.manifest_path(game, id);
        fs::create_dir_all(path.parent().expect("manifest parent")).expect("game directory");
        fs::write(path, text).expect("manifest");
    }

    pub fn manifest_path(&self, game: &str, id: &str) -> PathBuf {
        self.catalog
            .join("mods")
            .join(game)
            .join(format!("{id}.json"))
    }

    pub fn run_plain(&self, env: &[(&str, &str)], args: &[&str]) -> Output {
        self.command(env).args(args).output().expect("run CLI")
    }

    pub fn run(&self, env: &[(&str, &str)], args: &[&str]) -> Output {
        let mut command = self.command(env);
        command
            .args(args)
            .arg(&self.catalog)
            .output()
            .expect("run CLI")
    }

    pub fn run_target(&self, env: &[(&str, &str)], args: &[&str], target: &str) -> Output {
        self.command(env)
            .args(args)
            .arg(&self.catalog)
            .arg(target)
            .output()
            .expect("run CLI")
    }

    fn command(&self, env: &[(&str, &str)]) -> Command {
        let mut command = Command::new(main_bucket::modn_binary());
        command
            .env("LOCALAPPDATA", self.data.path())
            .env("APPDATA", self.data.path())
            .env("MODN_APPDATA", self.data.path())
            .arg("--state-dir")
            .arg(self.temp.path().join("state"));
        for (name, value) in env {
            command.env(name, value);
        }
        command
    }
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

pub fn json_output(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("one JSON object on stdout")
}

pub fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn collect(root: &Path, directory: &Path, output: &mut Vec<(String, Vec<u8>)>) {
        let mut entries: Vec<_> = fs::read_dir(directory)
            .expect("snapshot entries")
            .collect::<io::Result<Vec<_>>>()
            .expect("snapshot entries");
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            if entry.file_type().expect("snapshot type").is_dir() {
                collect(root, &path, output);
            } else {
                let name = path
                    .strip_prefix(root)
                    .expect("snapshot path")
                    .to_string_lossy()
                    .replace('\\', "/");
                output.push((name, fs::read(&path).expect("snapshot bytes")));
            }
        }
    }
    let mut output = Vec::new();
    collect(root, root, &mut output);
    output
}

pub fn archive_manifest(
    game: &str,
    id: &str,
    version: &str,
    source: Value,
    checkver: Option<Value>,
) -> Value {
    let mut manifest = json!({
        "schema_version": 2,
        "id": id,
        "game": game,
        "name": format!("Fixture {id}"),
        "version": version,
        "archive_format": "zip",
        "blake3": "0".repeat(64),
        "sources": [source],
        "dependencies": []
    });
    if let Some(checkver) = checkver {
        manifest["checkver"] = checkver;
    }
    manifest
}

pub fn git_manifest(game: &str, id: &str, version: &str, commit: &str, checkver: Value) -> Value {
    json!({
        "schema_version": 2,
        "id": id,
        "game": game,
        "name": format!("Fixture {id}"),
        "version": version,
        "sources": [{"kind": "git", "repository": "fixture/project", "commit": commit}],
        "dependencies": [],
        "checkver": checkver
    })
}

pub fn github_source(repository: &str, tag: &str, asset: &str) -> Value {
    json!({"kind": "github-release", "repository": repository, "tag": tag, "asset": asset})
}

pub fn github_route(repository: &str, body: Value) -> (String, (u16, Vec<u8>)) {
    (
        format!("/repos/{repository}/releases?per_page=30"),
        (200, serde_json::to_vec(&body).expect("route body")),
    )
}
