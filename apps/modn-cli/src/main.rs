mod buckets;
mod catalog_install;
mod commands;
mod modules;
mod selection;
mod state_access;
use anyhow::{bail, Context, Result};
use clap::Parser;
use commands::{Cli, Command, GameCommand};
use modn_core::archive::inspect;
use modn_core::game_modules::with_module_source;
use modn_core::inventory::{doctor_with_ownership_for, scan_with_ownership_for};
use modn_core::planning::preview_install_zip_with_permissions;
use modn_core::state::Store;
use selection::{choose_candidate, selected_installation};
use state_access::{database_path, validate_state_path};
use std::path::PathBuf;

fn run() -> Result<()> {
    let cli = Cli::parse();
    let state_path = database_path(cli.state_dir.as_deref())?;
    let module_source = cli.module_source.clone();
    with_module_source(module_source.as_deref(), || run_cli(cli, state_path))
}

fn run_cli(cli: Cli, state_path: PathBuf) -> Result<()> {
    if let Command::Bucket { command } = cli.command {
        return buckets::run(command, &state_path, cli.json, cli.game_id.as_deref());
    }
    if let Command::Module { command } = cli.command {
        return modules::run(command, cli.json);
    }
    if let Command::Search { query } = &cli.command {
        return buckets::search(
            query.as_deref(),
            &state_path,
            cli.json,
            cli.game_id.as_deref(),
        );
    }
    if let Command::Install {
        target,
        path,
        file,
        yes,
        dry_run,
        allow_executable,
    } = &cli.command
    {
        let source = std::path::Path::new(target);
        let is_archive = source.is_file()
            || source
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|ext| {
                    ["zip", "7z", "jar"].contains(&ext.to_ascii_lowercase().as_str())
                });
        if !is_archive {
            if !allow_executable.is_empty() {
                bail!("--allow-executable applies only to standalone archive installation");
            }
            return catalog_install::run(
                target,
                path.as_deref(),
                file.as_deref(),
                *yes,
                *dry_run,
                &state_path,
                cli.game_id.as_deref(),
                cli.json,
            );
        }
        if file.is_some() {
            bail!("--file applies only to cataloged mods, not standalone archives");
        }
    }
    let requested = cli.game_id.as_deref();
    if let Command::Game {
        command: GameCommand::Add { game, .. } | GameCommand::Remove { game, .. },
    } = &cli.command
    {
        if requested.is_some_and(|id| id != game) {
            bail!("--game must match the game ID passed to game add/remove");
        }
    }
    let selected_path = match &cli.command {
        Command::Game {
            command: GameCommand::Add { path, .. },
        }
        | Command::Install { path, .. }
        | Command::List { path }
        | Command::Uninstall { path, .. }
        | Command::Scan { path }
        | Command::Doctor { path } => path.as_deref(),
        _ => None,
    };

    // Before opening writable state, establish the selected game identity and
    // root using only the existing read-only registration database.
    let registered_only = matches!(
        &cli.command,
        Command::Install { path: None, .. }
            | Command::List { path: None }
            | Command::Uninstall { path: None, .. }
            | Command::Scan { path: None }
            | Command::Doctor { path: None }
    );
    // Reads do not create an empty manager database. Existing registrations
    // provide concrete physical roots for checks before any writable open.
    let standalone = matches!(
        &cli.command,
        Command::Inspect { .. } | Command::Module { .. }
    );
    let existing_state = if !standalone && state_path.is_file() {
        Some(Store::open_readonly(&state_path)?)
    } else {
        None
    };
    let registered_roots: Vec<PathBuf> = existing_state
        .as_ref()
        .map(Store::list)
        .transpose()?
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| entry.path.canonicalize().ok())
        .collect();
    let preselected = if registered_only {
        Some(selected_installation(
            existing_state.as_ref(),
            None,
            requested,
        )?)
    } else {
        None
    };
    let registration_candidate = match &cli.command {
        Command::Game {
            command: GameCommand::Add { game, path },
        } => Some(choose_candidate(game, path.clone())?),
        _ => None,
    };
    let state_game = match &cli.command {
        Command::Game {
            command: GameCommand::Add { game, .. } | GameCommand::Remove { game, .. },
        } => Some(game.as_str()),
        _ => requested.or_else(|| preselected.as_ref().map(|(id, _)| id.as_str())),
    };
    let protected_root = selected_path
        .or_else(|| preselected.as_ref().map(|(_, path)| path.as_path()))
        .or_else(|| {
            registration_candidate
                .as_ref()
                .map(|candidate| candidate.path.as_path())
        });
    let explicit_preview = matches!(
        &cli.command,
        Command::Install {
            dry_run: true,
            path: Some(_),
            ..
        }
    );
    let read_only_preview = matches!(&cli.command, Command::Install { dry_run: true, .. });
    if matches!(
        &cli.command,
        Command::Game {
            command: GameCommand::Remove { .. }
        }
    ) && existing_state.is_none()
    {
        bail!("no game registrations to remove");
    }
    let mut store = if matches!(
        &cli.command,
        Command::Inspect { .. } | Command::Module { .. }
    ) {
        None
    } else if matches!(
        &cli.command,
        Command::Game {
            command: GameCommand::List
        }
    ) {
        existing_state
    } else if explicit_preview && existing_state.is_none() {
        None
    } else {
        if matches!(&cli.command, Command::Recover { .. }) && existing_state.is_none() {
            bail!("no manager state database to recover");
        }
        validate_state_path(&state_path, state_game, protected_root, &registered_roots)?;
        Some(if read_only_preview {
            existing_state.context("no registered game database for preview")?
        } else {
            Store::open(&state_path)?
        })
    };
    match cli.command {
        Command::Search { .. } => unreachable!("search commands return before manager access"),
        Command::Module { .. } => unreachable!("module commands return before manager access"),
        Command::Bucket { .. } => unreachable!("bucket commands return before manager access"),
        Command::Install {
            target,
            path,
            file: _,
            yes,
            dry_run,
            allow_executable,
        } => {
            let archive = PathBuf::from(target);
            if !yes && !dry_run {
                bail!("installation writes game files; choose --dry-run or explicitly approve with --yes");
            }
            let (game_id, game) = selected_installation(store.as_ref(), path, requested)?;
            let report =
                preview_install_zip_with_permissions(&game_id, &game, &archive, &allow_executable)?;
            if dry_run {
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&report)?);
                } else {
                    println!("Dry run: {} (metadata only)", report.status);
                    for file in report.operations {
                        println!("{} => {}", file.source, file.destination);
                    }
                    for issue in report.blockers {
                        println!("BLOCKER {}: {}", issue.code, issue.message);
                    }
                }
            } else {
                let package = store
                    .as_mut()
                    .expect("install command has store")
                    .install_operations_zip_with_permissions(
                        &game,
                        &archive,
                        &report,
                        &allow_executable,
                    )?;
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&package)?);
                } else {
                    println!(
                        "Installed {} ({} owned files)",
                        package.id,
                        package.files.len()
                    );
                }
            }
        }
        Command::List { path } => {
            let (_, game) = selected_installation(store.as_ref(), path, requested)?;
            let packages = store
                .as_ref()
                .expect("non-inspect command has store")
                .installed(&game)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&packages)?);
            } else if packages.is_empty() {
                println!("No manager-owned packages installed.");
            } else {
                for package in packages {
                    println!("{}: {} owned files", package.id, package.files.len());
                }
            }
        }
        Command::Uninstall {
            package_id,
            path,
            yes,
        } => {
            if !yes {
                bail!("uninstall removes game files; rerun with --yes");
            }
            let (_, game) = selected_installation(store.as_ref(), path, requested)?;
            store
                .as_mut()
                .expect("non-inspect command has store")
                .uninstall(&game, &package_id)?;
            if cli.json {
                println!("{}", serde_json::json!({"uninstalled": package_id}));
            } else {
                println!("Uninstalled {package_id}");
            }
        }
        Command::Recover { yes } => {
            if !yes {
                bail!("recovery may roll back interrupted game changes; rerun with --yes");
            }
            let count = store
                .as_mut()
                .expect("non-inspect command has store")
                .recover()?;
            if cli.json {
                println!("{}", serde_json::json!({"recovered": count}));
            } else {
                println!("Recovered {count} interrupted transactions.");
            }
        }
        Command::Inspect { archive } => {
            let report = inspect(&archive)?;
            if cli.json {
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
        }
        Command::Game {
            command: GameCommand::Add { game, path: _ },
        } => {
            let id = game.as_str();
            let candidate = registration_candidate
                .as_ref()
                .context("game add candidate already validated")?;
            let entry = store
                .as_ref()
                .expect("non-inspect command has store")
                .add_game(id, &candidate.path, &candidate.source)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&entry)?);
            } else {
                println!("Registered {} ({})", entry.path.display(), entry.source);
            }
        }
        Command::Game {
            command: GameCommand::Remove { game, path },
        } => {
            let removed = store
                .as_mut()
                .context("game removal requires manager state")?
                .remove_game(&game, path.as_deref())?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&removed)?);
            } else {
                println!(
                    "Unregistered {} ({})",
                    removed.game_id,
                    removed.path.display()
                );
            }
        }
        Command::Game {
            command: GameCommand::List,
        } => {
            let entries = store
                .as_ref()
                .map(Store::list)
                .transpose()?
                .unwrap_or_default();
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&entries)?);
            } else if entries.is_empty() {
                println!("No game installations registered.");
            } else {
                for entry in entries {
                    println!(
                        "{}: {} ({})",
                        entry.game_id,
                        entry.path.display(),
                        entry.source
                    );
                }
            }
        }
        Command::Scan { path } => {
            let (game_id, path) = selected_installation(store.as_ref(), path, requested)?;
            let report = scan_with_ownership_for(
                store.as_ref().expect("non-inspect command has store"),
                &game_id,
                &path,
            )?;
            if report.incomplete {
                eprintln!("warning: modn scan incomplete (see doctor diagnostics)");
            }
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("{}: {}", game_id, report.installation.display());
                println!("REDmod executable present: {}", report.redmod_available);
                for entry in report.mods {
                    println!(
                        "{}: {} [{}{}]",
                        entry.kind,
                        entry.relative_path.display(),
                        if entry.managed {
                            "managed"
                        } else {
                            "unmanaged"
                        },
                        entry
                            .integrity
                            .as_deref()
                            .map(|status| format!(", {status}"))
                            .unwrap_or_default()
                    );
                }
                for file in report.owned_files {
                    if file.status != "intact" {
                        println!(
                            "OWNED {}: {} ({})",
                            file.status, file.relative_path, file.package_id
                        );
                    }
                }
                for signal in report.framework_signals {
                    println!("FRAMEWORK SIGNAL {}: {}", signal.name, signal.relative_path);
                }
            }
        }
        Command::Doctor { path } => {
            let (game_id, path) = selected_installation(store.as_ref(), path, requested)?;
            let report = doctor_with_ownership_for(
                store.as_ref().expect("non-inspect command has store"),
                &game_id,
                &path,
            )?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("{}: {}", game_id, report.scan.installation.display());
                println!(
                    "REDmod executable present: {}",
                    report.scan.redmod_available
                );
                println!(
                    "Detected unmanaged entries: {}",
                    report
                        .scan
                        .mods
                        .iter()
                        .filter(|entry| !entry.managed)
                        .count()
                );
                println!("Owned files checked: {}", report.scan.owned_files.len());
                for diagnostic in report.diagnostics {
                    println!(
                        "{} [{}]: {}",
                        diagnostic.severity, diagnostic.code, diagnostic.message
                    );
                }
            }
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
