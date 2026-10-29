mod selection;
mod state_access;
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use modn_core::archive::inspect;
use modn_core::game_modules::check_module_with_roots;
use modn_core::inventory::{doctor_with_ownership_for, scan_with_ownership_for};
use modn_core::planning::preview_install_zip;
use modn_core::state::Store;
use selection::{choose_candidate, selected_installation};
use state_access::{database_path, validate_state_path};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "modn", about = "Game mod inspection and guarded management")]
struct Cli {
    #[arg(long, global = true)]
    json: bool,

    #[arg(long, global = true, value_name = "DIR")]
    state_dir: Option<PathBuf>,

    /// Select a runtime-loaded game policy by ID.
    #[arg(long = "game", global = true, value_name = "ID")]
    game_id: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Preview or install a ZIP with the selected Luau policy after approval.
    Install {
        archive: PathBuf,
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long, help = "Confirm writing files into the game")]
        yes: bool,
        #[arg(
            long,
            conflicts_with = "yes",
            help = "Preview operations without writing game files"
        )]
        dry_run: bool,
    },
    /// List manager-owned packages (unmanaged mods are never included).
    Installed {
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Remove an unchanged manager-owned package.
    Uninstall {
        package_id: String,
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long, help = "Confirm removing managed game files")]
        yes: bool,
    },
    /// Safely roll back interrupted installs/uninstalls.
    Recover {
        #[arg(long, help = "Confirm rollback of interrupted transactions")]
        yes: bool,
    },
    /// Inspect ZIP, JAR, or 7z metadata without extracting files.
    Inspect { archive: PathBuf },
    /// Validate a local Luau module without granting file or game-write access.
    Module {
        #[command(subcommand)]
        command: ModuleCommand,
    },
    Game {
        #[command(subcommand)]
        command: GameCommand,
    },
    /// Inspect known mod directories without modifying game files.
    Scan {
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Check REDmod availability and report unmanaged content.
    Doctor {
        #[arg(long)]
        path: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum ModuleCommand {
    /// Load a local game module and report its identity and API compatibility.
    Check {
        directory: PathBuf,
        /// Explicitly permit read-only discovery and inspection within this directory.
        #[arg(long = "allow-root", value_name = "DIR")]
        allow_roots: Vec<PathBuf>,
    },
}

#[derive(Subcommand)]
enum GameCommand {
    /// Register an installation using its game's Luau module.
    Add {
        game: String,
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Show registered installations.
    List,
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let requested = cli.game_id.as_deref();
    if let Command::Game {
        command: GameCommand::Add { game, .. },
    } = &cli.command
    {
        if requested.is_some_and(|id| id != game) {
            bail!("--game must match the game ID passed to game add");
        }
    }
    let selected_path = match &cli.command {
        Command::Game {
            command: GameCommand::Add { path, .. },
        }
        | Command::Install { path, .. }
        | Command::Installed { path }
        | Command::Uninstall { path, .. }
        | Command::Scan { path }
        | Command::Doctor { path } => path.as_deref(),
        _ => None,
    };
    let state_path = database_path(cli.state_dir.as_deref())?;

    // Before opening writable state, establish the selected game identity and
    // root using only the existing read-only registration database.
    let registered_only = matches!(
        &cli.command,
        Command::Install { path: None, .. }
            | Command::Installed { path: None }
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
            command: GameCommand::Add { game, .. },
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
        Command::Module {
            command:
                ModuleCommand::Check {
                    directory,
                    allow_roots,
                },
        } => {
            let module = check_module_with_roots(&directory, &allow_roots)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&module)?);
            } else {
                println!(
                    "Game module {} v{} (host API {})",
                    module.game_id, module.version, module.api_version
                );
            }
        }
        Command::Install {
            archive,
            path,
            yes,
            dry_run,
        } => {
            if !yes && !dry_run {
                bail!("installation writes game files; choose --dry-run or explicitly approve with --yes");
            }
            let (game_id, game) = selected_installation(store.as_ref(), path, requested)?;
            let report = preview_install_zip(&game_id, &game, &archive)?;
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
                    .install_operations_zip(&game, &archive, &report)?;
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
        Command::Installed { path } => {
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
                eprintln!("warning: modn scan incomplete (entry limit reached)");
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
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
