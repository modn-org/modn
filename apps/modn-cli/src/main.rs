mod adoption;
mod assets;
mod buckets;
mod catalog_install;
mod commands;
mod dependency_plan;
mod download;
mod game_detect;
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
mod play;
mod progress;
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
use modn_core::error::{code_of, ErrorCode};
use modn_core::game_modules::with_module_source;
use modn_core::transactions::NativeInjectorNotAllowed;
use progress::Progress;
use state_access::database_path;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

fn run(mut cli: Cli, progress: &Progress) -> Result<()> {
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
        with_module_source(module_source.as_deref(), || {
            run_cli(cli, &state_path, progress)
        })
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

fn run_cli(cli: Cli, state_path: &Path, progress: &Progress) -> Result<()> {
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
        Command::Install(args) => install::run(&args, state_path, requested, json, progress),
        Command::Adopt(args) => adoption::run(&args, state_path, requested, json),
        Command::Update(args) => update::run(&args, state_path, requested, json, progress),
        Command::Pin(args) => managed::pin(&args, state_path, requested, true),
        Command::Unpin(args) => managed::pin(&args, state_path, requested, false),
        Command::Inspect { archive } => inspect_archive(&archive, json),
        Command::Game { command } => games::run(command, state_path, requested, json),
        Command::Play { game, path } => play::run(&game, path, state_path, requested, json),
        Command::List { path } => managed::list(path, state_path, requested, json),
        Command::Uninstall(args) => {
            managed::uninstall(&args, state_path, requested, json, progress)
        }
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

/// The stable code of a command that failed without a more specific one.
fn default_code(command: &Command) -> ErrorCode {
    match command {
        Command::Install(_) => ErrorCode::InstallFailed,
        Command::Adopt(_) => ErrorCode::AdoptFailed,
        Command::Update(_) => ErrorCode::UpdateFailed,
        Command::Pin(_) | Command::Unpin(_) => ErrorCode::PinFailed,
        Command::Search { .. } => ErrorCode::SearchFailed,
        Command::Cache { .. } => ErrorCode::CacheFailed,
        Command::Bucket { .. } => ErrorCode::BucketFailed,
        Command::List { .. } => ErrorCode::ListFailed,
        Command::Uninstall(_) => ErrorCode::UninstallFailed,
        Command::Recover { .. } => ErrorCode::RecoverFailed,
        Command::Inspect { .. } => ErrorCode::InspectFailed,
        Command::Asset { .. } => ErrorCode::AssetFailed,
        Command::Module { .. } => ErrorCode::ModuleFailed,
        Command::Auth { .. } => ErrorCode::AuthFailed,
        Command::Game { .. } | Command::Play { .. } => ErrorCode::GameFailed,
        Command::Scan { .. } => ErrorCode::ScanFailed,
        Command::Doctor { .. } => ErrorCode::DoctorFailed,
    }
}

/// Commands that report progress and accept cancellation.
fn reports_progress(command: &Command) -> bool {
    matches!(
        command,
        Command::Install(_) | Command::Update(_) | Command::Uninstall(_)
    )
}

fn main() {
    let cli = Cli::parse();
    let json = cli.json;
    let fallback = default_code(&cli.command);
    let progress = Progress::start(cli.progress.filter(|_| reports_progress(&cli.command)));
    let result = run(cli, &progress);
    if let Err(error) = result {
        let code = if error
            .downcast_ref::<catalog_install::AmbiguousTarget>()
            .is_some()
        {
            ErrorCode::AmbiguousTarget
        } else {
            code_of(&error).unwrap_or(fallback)
        };
        progress.finish_error(code);
        if let Some(ambiguous) = error.downcast_ref::<catalog_install::AmbiguousTarget>() {
            if ambiguous.json || progress.enabled() {
                eprintln!("{}", ambiguous.as_json());
                std::process::exit(1);
            }
        }
        if json || progress.enabled() {
            if let Some(refusal) = error.downcast_ref::<NativeInjectorNotAllowed>() {
                eprintln!("{}", refusal.as_json());
                std::process::exit(1);
            }
            let message = format!("{error:#}");
            eprintln!(
                "{}",
                serde_json::json!({"error": code.as_str(), "message": message})
            );
            std::process::exit(1);
        }
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
    progress.finish_success();
}
mod bucket_update;
