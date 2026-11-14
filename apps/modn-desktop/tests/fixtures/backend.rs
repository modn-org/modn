//! Disposable stand-in for `modn.exe`, used by the desktop contract tests.
//!
//! The fixture keeps its `mode`-file behaviours: `overflow`, `failure`,
//! `invalid-json`, `timeout`, `slow`, `blocked`, `registration-changed`,
//! `source-changed` and `inventory-failure`.
//!
//! When the arguments contain `--progress json` it also speaks the CLI's
//! progress protocol: one JSON event per line on stderr and `cancel` lines on
//! stdin. A scenario name in the arguments after the fixed prefix selects the
//! behaviour:
//!
//! - `hang-before-commit`: emits `resolve`, `download`, `extract` and `stage`,
//!   then waits for `cancel` and exits 1 with the CLI's `CANCELLED` envelope.
//!   It writes nothing.
//! - `hang-after-commit`: emits through `commit`, ignores `cancel`, waits 1.5
//!   seconds, records the write and succeeds.
//! - `stall-after-cancel`: emits through `stage`, then ignores `cancel` and
//!   stalls for 30 seconds before exiting 2 without a write.
//! - `oversized-after-commit`: commits, records the write, writes 2 MiB on
//!   stdout, records that it survived that write, then exits 0.
//! - `slow`: emits each phase 250 ms apart, checks for `cancel` before the
//!   commit, then records the write and succeeds.
//! - `refuse`: emits plain stderr text and a `PLAN_BLOCKED` envelope, then
//!   exits 1 without a write.
//! - any other name: emits the full sequence and succeeds.

use serde_json::json;
use std::ffi::OsString;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "--fixture-grandchild")
    {
        thread::sleep(Duration::from_secs(4));
        return Ok(());
    }
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    anyhow::ensure!(
        args.first().is_some_and(|arg| arg == "--json"),
        "JSON required"
    );
    anyhow::ensure!(
        args.get(1).is_some_and(|arg| arg == "--state-dir"),
        "State required"
    );
    let state = PathBuf::from(args.get(2).expect("fixed backend state argument"));
    anyhow::ensure!(
        state.join("test-backend.marker").is_file(),
        "Disposable state marker required"
    );
    let mode = fs::read_to_string(state.join("mode")).unwrap_or_default();
    if mode == "inventory-failure" && state.join("write.json").exists() {
        eprintln!("fixture inventory unavailable after completed write");
        std::process::exit(42);
    }
    fs::write(
        state.join("last-args.json"),
        serde_json::to_vec(
            &args
                .iter()
                .map(|arg| arg.to_string_lossy())
                .collect::<Vec<_>>(),
        )?,
    )?;
    fs::write(state.join("child-pid"), std::process::id().to_string())?;
    if args.iter().any(|arg| arg == "--record-env") {
        let names = [
            "SystemRoot",
            "TEMP",
            "TMP",
            "LOCALAPPDATA",
            "MODN_CACHE",
            "MODN_STORE_LIBRARIES",
            "MODN_APPDATA",
            "MODN_GAME_MODULES_DIR",
            "PATH",
        ];
        let record: serde_json::Map<String, serde_json::Value> = names
            .iter()
            .map(|name| ((*name).to_owned(), json!(std::env::var_os(name).is_some())))
            .collect();
        fs::write(state.join("environment.json"), serde_json::to_vec(&record)?)?;
    }
    if args.iter().any(|arg| arg == "--progress") {
        return progress(&args, &state);
    }
    if let Some(scenario) = fixture_scenario(&args) {
        return registration_scenario(scenario, &state);
    }
    match mode.as_str() {
        "overflow" => {
            io::stdout().write_all(&vec![b'x'; 2 * 1024 * 1024])?;
            return Ok(());
        }
        "failure" => {
            eprintln!("fixture refusal");
            std::process::exit(41);
        }
        "invalid-json" => {
            println!("invalid fixture response");
            return Ok(());
        }
        "timeout" => std::thread::sleep(Duration::from_secs(120)),
        _ => {}
    }
    let tail: Vec<_> = args[3..].iter().map(|s| s.to_string_lossy()).collect();
    let game = json!({"game_id":"fixture", "path":state.join("game")});
    let result = if tail == ["game", "list"] {
        if mode == "registration-changed" {
            json!([])
        } else {
            json!([game])
        }
    } else if tail.iter().any(|s| s == "--dry-run") {
        json!({"status":"reviewable", "metadata_only":true,
            "operations":[{"source":"plugins/demo.plugin", "destination":"addons/demo.plugin"}],
            "blockers":if mode == "blocked" {json!([{"message":"blocked"}])} else {json!([])}})
    } else if tail.iter().any(|s| s == "inspect") {
        json!({"metadata_only":true})
    } else if tail.iter().any(|s| s == "list") {
        if mode == "slow" {
            std::thread::sleep(Duration::from_secs(12));
        }
        json!([{"id":"owned-package", "files":["addons/demo.plugin"]}])
    } else if tail.iter().any(|s| s == "--yes") {
        record_write(&args, &state)?;
        json!({"id":"owned-package"})
    } else {
        json!({})
    };
    println!("{result}");
    Ok(())
}

fn progress(args: &[OsString], state: &Path) -> anyhow::Result<()> {
    let scenario = scenario(args);
    let cancel = Arc::new(AtomicBool::new(false));
    let reader = Arc::clone(&cancel);
    thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if line.trim() == "cancel" {
                reader.store(true, Ordering::SeqCst);
            }
        }
    });
    emit(json!({"phase":"resolve"}));
    sleep_if_slow(scenario);
    emit(json!({"phase":"download","package":"fixture-package","bytes":512,"total":1024}));
    sleep_if_slow(scenario);
    emit(json!({"phase":"extract","package":"fixture-package"}));
    sleep_if_slow(scenario);
    emit(json!({"phase":"stage","package":"fixture-package"}));
    if scenario == "refuse" {
        eprintln!("fixture stderr noise that callers must never see");
        emit(json!({"error":"PLAN_BLOCKED","message":"fixture secret refusal text"}));
        std::process::exit(1);
    }
    if scenario == "hang-before-commit" {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while !cancel.load(Ordering::SeqCst) {
            if std::time::Instant::now() >= deadline {
                emit(json!({"error":"FIXTURE_TIMEOUT","message":"no cancel received"}));
                std::process::exit(2);
            }
            thread::sleep(Duration::from_millis(10));
        }
        exit_cancelled();
    }
    if scenario == "stall-after-cancel" {
        // A step that started before the cancel cannot act on it. The desktop
        // must still end the call; this deadline only bounds the test.
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        emit(json!({"error":"FIXTURE_TIMEOUT","message":"the stalled cancel was never killed"}));
        std::process::exit(2);
    }
    sleep_if_slow(scenario);
    if scenario == "slow" && cancel.load(Ordering::SeqCst) {
        exit_cancelled();
    }
    emit(json!({"phase":"commit","package":"fixture-package"}));
    if scenario == "hang-after-commit" {
        thread::sleep(Duration::from_millis(1500));
        if cancel.load(Ordering::SeqCst) {
            fs::write(state.join("cancel-after-commit"), b"ignored")?;
        }
    }
    record_write(args, state)?;
    emit(json!({"phase":"result","outcome":"succeeded"}));
    if scenario == "oversized-after-commit" {
        let filler = vec![b'x'; 2 * 1024 * 1024];
        io::stdout().write_all(&filler)?;
        io::stdout().flush()?;
        fs::write(state.join("survived-after-output"), b"not killed")?;
        return Ok(());
    }
    println!("{}", json!({"id":"owned-package"}));
    Ok(())
}

fn fixture_scenario(args: &[OsString]) -> Option<&'static str> {
    let position = args.iter().position(|arg| arg == "--fixture-scenario")?;
    match args.get(position + 1)?.to_str()? {
        "heartbeat" => Some("heartbeat"),
        "silent" => Some("silent"),
        "committed-stall" => Some("committed-stall"),
        "game-launch" => Some("game-launch"),
        _ => None,
    }
}

fn registration_scenario(scenario: &str, state: &Path) -> anyhow::Result<()> {
    match scenario {
        "heartbeat" => {
            for _ in 0..10 {
                eprintln!("fixture heartbeat");
                thread::sleep(Duration::from_millis(100));
            }
        }
        "silent" => thread::sleep(Duration::from_millis(1500)),
        "committed-stall" => {
            println!("{}", json!({"game_id": "fixture"}));
            io::stdout().flush()?;
            fs::write(state.join("committed"), b"yes")?;
            thread::sleep(Duration::from_millis(1200));
            fs::write(state.join("survived-after-result"), b"yes")?;
            return Ok(());
        }
        "game-launch" => {
            let executable = std::env::current_exe()?;
            std::process::Command::new(executable)
                .arg("--fixture-grandchild")
                .spawn()?;
        }
        _ => {}
    }
    println!("{}", json!({"game_id": "fixture"}));
    Ok(())
}

fn scenario(args: &[OsString]) -> &'static str {
    let tail = args.get(3..).unwrap_or_default();
    for name in [
        "hang-before-commit",
        "hang-after-commit",
        "stall-after-cancel",
        "oversized-after-commit",
        "refuse",
        "slow",
    ] {
        if tail.iter().any(|arg| arg.to_string_lossy().contains(name)) {
            return name;
        }
    }
    "complete"
}

fn sleep_if_slow(scenario: &str) {
    if scenario == "slow" {
        thread::sleep(Duration::from_millis(250));
    }
}

fn exit_cancelled() -> ! {
    emit(json!({"phase":"result","outcome":"cancelled","error":"CANCELLED"}));
    emit(
        json!({"error":"CANCELLED","message":"operation cancelled before the commit point; nothing changed"}),
    );
    std::process::exit(1);
}

fn emit(value: serde_json::Value) {
    eprintln!("{value}");
}

fn record_write(args: &[OsString], state: &Path) -> anyhow::Result<()> {
    fs::write(
        state.join("write.json"),
        serde_json::to_vec(
            &args
                .iter()
                .map(|arg| arg.to_string_lossy())
                .collect::<Vec<_>>(),
        )?,
    )?;
    Ok(())
}
