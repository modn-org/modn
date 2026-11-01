mod buckets;
mod catalog_install;
mod commands;
mod games;
mod install;
mod inventory;
mod managed;
mod modules;
mod selection;
mod state_access;
use anyhow::Result;
use clap::Parser;
use commands::{Cli, Command};
use modn_core::archive::inspect;
use modn_core::game_modules::with_module_source;
use state_access::database_path;
use std::path::Path;

fn run() -> Result<()> {
    let cli = Cli::parse();
    let state_path = database_path(cli.state_dir.as_deref())?;
    let module_source = cli.module_source.clone();
    with_module_source(module_source.as_deref(), || run_cli(cli, &state_path))
}

// Each handler decides its own manager-state access through `ManagerState`.
fn run_cli(cli: Cli, state_path: &Path) -> Result<()> {
    let Cli {
        json,
        game_id,
        command,
        ..
    } = cli;
    let requested = game_id.as_deref();
    match command {
        Command::Bucket { command } => buckets::run(command, state_path, json, requested),
        Command::Module { command } => modules::run(command, json),
        Command::Search { query } => buckets::search(query.as_deref(), state_path, json, requested),
        Command::Install(args) => install::run(&args, state_path, requested, json),
        Command::Inspect { archive } => inspect_archive(&archive, json),
        Command::Game { command } => games::run(command, state_path, requested, json),
        Command::List { path } => managed::list(path, state_path, requested, json),
        Command::Uninstall {
            package_id,
            path,
            yes,
        } => managed::uninstall(&package_id, path, yes, state_path, requested, json),
        Command::Recover { yes } => managed::recover(yes, state_path, requested, json),
        Command::Scan { path } => inventory::scan(path, state_path, requested, json),
        Command::Doctor { path } => inventory::doctor(path, state_path, requested, json),
    }
}

// Inspection reads only the archive; it never opens manager state.
fn inspect_archive(archive: &Path, json: bool) -> Result<()> {
    let report = inspect(archive)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "{} ({}, metadata only)",
            report.archive.display(),
            report.format
        );
        println!("SHA-256: {}", report.sha256);
        for file in report.files {
            println!("{} ({} bytes)", file.entry, file.bytes);
        }
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        if let Some(ambiguous) = error.downcast_ref::<catalog_install::AmbiguousTarget>() {
            if ambiguous.json {
                eprintln!("{}", ambiguous.as_json());
                std::process::exit(1);
            }
        }
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
