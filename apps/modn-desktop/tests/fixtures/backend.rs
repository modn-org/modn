use serde_json::json;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;

fn main() -> anyhow::Result<()> {
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
            "operations":[{"destination":"addons/demo.plugin"}],
            "blockers":if mode == "blocked" {json!([{"message":"blocked"}])} else {json!([])}})
    } else if tail.iter().any(|s| s == "inspect") {
        json!({"metadata_only":true})
    } else if tail.iter().any(|s| s == "list") {
        if mode == "slow" {
            std::thread::sleep(Duration::from_secs(12));
        }
        json!([{"id":"owned-package", "files":["addons/demo.plugin"]}])
    } else if tail.iter().any(|s| s == "--yes") {
        fs::write(
            state.join("write.json"),
            serde_json::to_vec(
                &args
                    .iter()
                    .map(|arg| arg.to_string_lossy())
                    .collect::<Vec<_>>(),
            )?,
        )?;
        json!({"id":"owned-package"})
    } else {
        json!({})
    };
    println!("{result}");
    Ok(())
}
