use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn event(line: &str) {
    eprintln!("{line}");
}

fn cancelled_before(flag: &AtomicBool) -> bool {
    for _ in 0..30 {
        if flag.load(Ordering::SeqCst) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
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
        "stderr" => {
            io::stderr()
                .write_all(&vec![b'x'; 300_000])
                .expect("output");
        }
        "sleep" => {
            std::fs::write(state.join("pid"), std::process::id().to_string()).expect("pid");
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
        "fail" => std::process::exit(7),
        "progress" => {
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
            std::thread::sleep(std::time::Duration::from_millis(400));
            event(r#"{"phase":"result","outcome":"succeeded"}"#);
            println!("{}", r#"{"fixture":"progress"}"#);
        }
        _ => println!("invalid backend JSON"),
    }
}
