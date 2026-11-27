mod adoption;
mod assets;
mod buckets;
mod catalog;
mod catalog_approval;
mod catalog_install;
mod commands;
mod conflicts;
mod dependency_plan;
mod download;
mod game_detect;
mod games;
mod git_source;
mod history;
mod install;
mod install_consent;
mod install_report;
mod inventory;
mod main_key;
mod main_release;
mod managed;
mod module_releases;
mod modules;
mod nexus_auth;
#[cfg(test)]
mod nexus_fixture_tests;
mod nxm;
mod play;
mod profile;
mod progress;
#[cfg(test)]
mod provider_fixture_tests;
mod selection;
mod state_access;
mod themes;
mod undo;
mod update;
mod update_refresh;
mod update_report;
use anyhow::{bail, Context, Result};
use clap::Parser;
use commands::{Cli, Command};
use modn_core::archive::inspect;
use modn_core::error::{code_of, ErrorCode};
use modn_core::manager_roots::ManagerRoots;
use modn_core::transactions::NativeInjectorNotAllowed;
use progress::Progress;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn manager_roots(cli: &Cli) -> Result<ManagerRoots> {
    let mut roots = ManagerRoots::from_environment(cli.state_dir.as_deref())?;
    if let Some(directory) = cli.module_source.clone() {
        roots = roots.with_development_modules(Some(directory));
    } else if let Some(base) = std::env::var_os("MODN_GAME_MODULES_DIR") {
        roots = roots.with_development_module_base(Some(PathBuf::from(base)));
    }
    if roots.profile().as_os_str().is_empty() {
        bail!("invalid manager state directory");
    }
    Ok(roots)
}

fn run(mut cli: Cli, progress: &Progress) -> Result<()> {
    if let Command::Asset { command } = &cli.command {
        return assets::run(command, cli.json);
    }
    if let Command::Auth { command } = &cli.command {
        return nexus_auth::run(command, cli.json);
    }
    if let Command::Catalog { command } = &cli.command {
        return catalog::run(command, cli.json);
    }
    if let Command::Theme { command } = &cli.command {
        let roots = manager_roots(&cli)?;
        return themes::run(command, &roots, cli.game_id.as_deref(), cli.json);
    }
    prepare_nxm(&mut cli.command)?;
    let roots = manager_roots(&cli)?;
    run_cli(cli, &roots, progress)
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

fn run_cli(cli: Cli, roots: &ManagerRoots, progress: &Progress) -> Result<()> {
    let Cli {
        json,
        game_id,
        command,
        ..
    } = cli;
    let requested = game_id.as_deref();
    match command {
        Command::Asset { command } => assets::run(&command, json),
        Command::Bucket { command } => buckets::run(command, roots, json, requested),
        Command::Cache { command } => download::run(command, roots, json),
        Command::Module { command } => modules::run(command, roots, json),
        Command::Auth { command } => nexus_auth::run(&command, json),
        Command::Search { query } => buckets::search(query.as_deref(), roots, json, requested),
        Command::Install(args) => install::run(&args, roots, requested, json, progress),
        Command::Adopt(args) => adoption::run(&args, roots, requested, json),
        Command::Update(args) => update::run(&args, roots, requested, json, progress),
        Command::Pin(args) => managed::pin(&args, roots, requested, true),
        Command::Unpin(args) => managed::pin(&args, roots, requested, false),
        Command::Inspect { archive } => inspect_archive(&archive, json),
        Command::Game { command } => games::run(command, roots, requested, json),
        Command::Play { game, path } => play::run(&game, path, roots, requested, json),
        Command::List { path } => managed::list(path, roots, requested, json),
        Command::History => history::run(roots, requested, json),
        Command::Uninstall(args) => managed::uninstall(&args, roots, requested, json, progress),
        Command::Undo(args) => undo::run(&args, roots, requested, json, progress),
        Command::Recover { yes } => managed::recover(yes, roots, requested, json),
        Command::Scan { path } => inventory::scan(path, roots, requested, json),
        Command::Doctor { path } => inventory::doctor(path, roots, requested, json),
        Command::Catalog { command } => catalog::run(&command, json),
        Command::Profile { command } => profile::run(&command, roots, requested, json),
        Command::Conflicts(args) => conflicts::run(&args, roots, requested, json),
        Command::Theme { .. } => bail!("theme commands run before manager state is opened"),
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
        println!("BLAKE3: {}", report.blake3);
        for file in report.files {
            println!("{} ({} bytes)", file.entry, file.bytes);
        }
    }
    Ok(())
}

fn default_code(command: &Command) -> ErrorCode {
    match command {
        Command::Install(_) => ErrorCode::InstallFailed,
        Command::Adopt(_) => ErrorCode::AdoptFailed,
        Command::Update(_) => ErrorCode::UpdateFailed,
        Command::Pin(_) | Command::Unpin(_) => ErrorCode::PinFailed,
        Command::Search { .. } => ErrorCode::SearchFailed,
        Command::Cache { .. } => ErrorCode::CacheFailed,
        Command::Bucket { .. } => ErrorCode::BucketFailed,
        Command::Catalog { .. } => ErrorCode::BucketFailed,
        Command::Profile { .. } => ErrorCode::ProfileFailed,
        Command::Conflicts(_) => ErrorCode::ConflictsFailed,
        Command::Theme { .. } => ErrorCode::InvalidArguments,
        Command::List { .. } => ErrorCode::ListFailed,
        Command::History => ErrorCode::HistoryFailed,
        Command::Uninstall(_) => ErrorCode::UninstallFailed,
        Command::Undo(_) => ErrorCode::UndoFailed,
        Command::Recover { .. } => ErrorCode::RecoverFailed,
        Command::Inspect { .. } => ErrorCode::InspectFailed,
        Command::Asset { .. } => ErrorCode::AssetFailed,
        Command::Module { .. } => ErrorCode::ModuleFailed,
        Command::Auth { .. } => ErrorCode::AuthFailed,
        Command::Game { .. } => ErrorCode::GameFailed,
        Command::Play { .. } => ErrorCode::PlayFailed,
        Command::Scan { .. } => ErrorCode::ScanFailed,
        Command::Doctor { .. } => ErrorCode::DoctorFailed,
    }
}

fn reports_progress(command: &Command) -> bool {
    matches!(
        command,
        Command::Install(_) | Command::Update(_) | Command::Uninstall(_) | Command::Undo(_)
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
            if let Some(refusal) = error.downcast_ref::<modn_core::approval::ApprovalRefusal>() {
                eprintln!("{}", refusal.as_json());
                std::process::exit(1);
            }
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
