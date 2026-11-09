mod adoption;
mod assets;
mod buckets;
mod catalog_install;
mod commands;
mod dependency_plan;
mod download;
mod games;
mod git_source;
mod install;
mod install_consent;
mod inventory;
mod main_key;
mod managed;
mod module_releases;
mod modules;
mod nexus_auth;
#[cfg(test)]
mod nexus_fixture_tests;
mod nxm;
#[cfg(test)]
mod provider_fixture_tests;
mod selection;
mod stack_install;
mod state_access;
mod update;
mod update_refresh;
mod update_report;
use anyhow::{Context, Result};
use clap::Parser;
use commands::{Cli, Command};
use modn_core::archive::inspect;
use modn_core::data_dir::with_profile_directory;
use modn_core::game_modules::with_module_source;
use modn_core::transactions::NativeInjectorNotAllowed;
use state_access::database_path;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

fn run(mut cli: Cli) -> Result<()> {
    if let Command::Asset { command } = &cli.command {
        return assets::run(command, cli.json);
    }
    if let Command::Auth { command } = &cli.command {
        return nexus_auth::run(command, cli.json);
    }
    prepare_nxm(&mut cli.command)?;
    let state_path = database_path(cli.state_dir.as_deref())?;
    let profile = state_path
        .parent()
        .context("invalid manager state directory")?;
    let module_source = cli.module_source.clone();
    with_profile_directory(profile, || {
        with_module_source(module_source.as_deref(), || run_cli(cli, &state_path))
    })
}

fn prepare_nxm(command: &mut Command) -> Result<()> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_secs();
    match command {
        Command::Install(args) => args.nxm_links = nxm::parse_all(&args.nxm, now)?,
        Command::Update(args) if args.target.is_some() => {
            args.nxm_links = nxm::parse_all(&args.nxm, now)?;
        }
        _ => {}
    }
    Ok(())
}

fn run_cli(cli: Cli, state_path: &Path) -> Result<()> {
    let Cli {
        json,
        game_id,
        command,
        ..
    } = cli;
    let requested = game_id.as_deref();
    match command {
        Command::Asset { command } => assets::run(&command, json),
        Command::Bucket { command } => buckets::run(command, state_path, json, requested),
        Command::Cache { command } => download::run(command, state_path, json),
        Command::Module { command } => modules::run(command, state_path, json),
        Command::Auth { command } => nexus_auth::run(&command, json),
        Command::Search { query } => buckets::search(query.as_deref(), state_path, json, requested),
        Command::Install(args) => install::run(&args, state_path, requested, json),
        Command::Adopt(args) => adoption::run(&args, state_path, requested, json),
        Command::Update(args) => update::run(&args, state_path, requested, json),
        Command::Pin(args) => managed::pin(&args, state_path, requested, true),
        Command::Unpin(args) => managed::pin(&args, state_path, requested, false),
        Command::Inspect { archive } => inspect_archive(&archive, json),
        Command::Game { command } => games::run(command, state_path, requested, json),
        Command::List { path } => managed::list(path, state_path, requested, json),
        Command::Uninstall(args) => managed::uninstall(&args, state_path, requested, json),
        Command::Recover { yes } => managed::recover(yes, state_path, requested, json),
        Command::Scan { path } => inventory::scan(path, state_path, requested, json),
        Command::Doctor { path } => inventory::doctor(path, state_path, requested, json),
    }
}

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

fn error_code(message: &str) -> &str {
    let code = message.split(':').next().unwrap_or_default();
    if code.len() >= 3
        && code
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        code
    } else {
        "ERROR"
    }
}

fn main() {
    let cli = Cli::parse();
    let json = cli.json;
    if let Err(error) = run(cli) {
        if let Some(ambiguous) = error.downcast_ref::<catalog_install::AmbiguousTarget>() {
            if ambiguous.json {
                eprintln!("{}", ambiguous.as_json());
                std::process::exit(1);
            }
        }
        if json {
            if let Some(refusal) = error.downcast_ref::<NativeInjectorNotAllowed>() {
                eprintln!("{}", refusal.as_json());
                std::process::exit(1);
            }
            let message = format!("{error:#}");
            let code = error_code(&message).to_owned();
            eprintln!("{}", serde_json::json!({"error": code, "message": message}));
            std::process::exit(1);
        }
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
mod bucket_update;
