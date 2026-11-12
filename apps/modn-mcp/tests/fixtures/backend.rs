use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn event(line: &str) {
    eprintln!("{line}");
}

fn cancelled_before(flag: &AtomicBool) -> bool {
    for _ in 0..30 {
        if flag.load(Ordering::SeqCst) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// Watches stdin for the companion's `cancel` line.
fn watch_cancellation() -> Arc<AtomicBool> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancelled);
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if line.trim() == "cancel" {
                flag.store(true, Ordering::SeqCst);
            }
        }
    });
    cancelled
}

fn record_invocation(state: &Path) {
    let arguments: Vec<_> = std::env::args_os()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(state.join("invocations"))
    {
        let _ = writeln!(file, "{}", arguments.join(" "));
    }
}

fn progress(state: &Path, markers: bool) {
    let cancelled = watch_cancellation();
    event(r#"{"phase":"resolve"}"#);
    if cancelled_before(&cancelled) {
        event(r#"{"phase":"result","outcome":"cancelled","error":"CANCELLED"}"#);
        std::process::exit(1);
    }
    event(r#"{"phase":"download","package":"fixture","bytes":0,"total":64}"#);
    event(r#"{"phase":"download","package":"fixture","bytes":64,"total":64}"#);
    event(r#"{"phase":"extract","package":"fixture"}"#);
    event(r#"{"phase":"stage","package":"fixture"}"#);
    if cancelled_before(&cancelled) {
        event(r#"{"phase":"result","outcome":"cancelled","error":"CANCELLED"}"#);
        std::process::exit(1);
    }
    event(r#"{"phase":"commit","package":"fixture"}"#);
    if markers {
        std::fs::write(state.join("commit"), b"committed").expect("commit marker");
    }
    std::thread::sleep(Duration::from_millis(400));
    event(r#"{"phase":"result","outcome":"succeeded"}"#);
    println!("{}", r#"{"fixture":"progress"}"#);
}

/// Writes the commit event in two chunks so a competing select branch can
/// cancel the line read between them.
fn split() {
    let cancelled = watch_cancellation();
    event(r#"{"phase":"resolve"}"#);
    let line = r#"{"phase":"commit","package":"fixture"}"#;
    let middle = line.len() / 2;
    let mut stderr = io::stderr();
    stderr
        .write_all(line[..middle].as_bytes())
        .expect("first chunk");
    stderr.flush().expect("first flush");
    std::thread::sleep(Duration::from_millis(1000));
    stderr
        .write_all(line[middle..].as_bytes())
        .expect("second chunk");
    stderr.write_all(b"\n").expect("newline");
    stderr.flush().expect("second flush");
    std::thread::sleep(Duration::from_millis(200));
    if cancelled.load(Ordering::SeqCst) {
        event(r#"{"phase":"result","outcome":"cancelled","error":"CANCELLED"}"#);
        std::process::exit(1);
    }
    event(r#"{"phase":"result","outcome":"succeeded"}"#);
    println!("{}", r#"{"fixture":"split"}"#);
}

fn burst() {
    event(r#"{"phase":"resolve"}"#);
    for index in 0..32u64 {
        event(&format!(
            r#"{{"phase":"download","package":"fixture","bytes":{index},"total":32}}"#
        ));
    }
    event(r#"{"phase":"result","outcome":"succeeded"}"#);
    println!("{}", r#"{"fixture":"burst"}"#);
}

fn many() {
    for index in 0..3000u64 {
        event(&format!(
            r#"{{"phase":"download","package":"fixture-progress-package-with-a-long-name-{index:04}","bytes":{index},"total":3000}}"#
        ));
    }
    println!("{}", r#"{"fixture":"many"}"#);
}

fn main() {
    let arguments: Vec<_> = std::env::args_os().collect();
    let index = arguments
        .iter()
        .position(|value| value == "--state-dir")
        .expect("state argument");
    let state = PathBuf::from(&arguments[index + 1]);
    let mode = std::fs::read_to_string(state.join("mode")).expect("fixture mode");
    match mode.as_str() {
        "large" => {
            io::stdout()
                .write_all(&vec![b'x'; 300_000])
                .expect("output");
        }
        "huge" => {
            let chunk = vec![b'x'; 64 * 1024];
            let mut stdout = io::stdout();
            for _ in 0..256 {
                if stdout.write_all(&chunk).is_err() {
                    break;
                }
            }
            let _ = stdout.flush();
            std::thread::sleep(Duration::from_secs(60));
        }
        "stderr" => {
            io::stderr()
                .write_all(&vec![b'x'; 300_000])
                .expect("output");
        }
        "sleep" => {
            std::fs::write(state.join("pid"), std::process::id().to_string()).expect("pid");
            std::thread::sleep(Duration::from_secs(60));
        }
        "fail" => std::process::exit(7),
        "progress" => progress(&state, false),
        "queued" => {
            record_invocation(&state);
            progress(&state, true);
        }
        "split" => split(),
        "burst" => burst(),
        "many" => many(),
        _ => println!("invalid backend JSON"),
    }
}
