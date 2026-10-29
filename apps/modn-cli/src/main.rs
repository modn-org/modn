use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use modn_core::archive::inspect;
use modn_core::discovery::{discover, validate_installation, Candidate};
use modn_core::game_modules::check_module;
use modn_core::inventory::{doctor_with_ownership, scan_with_ownership};
use modn_core::planning::plan;
use modn_core::state::Store;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "modn",
    about = "Cyberpunk mod inspection and guarded management"
)]
struct Cli {
    #[arg(long, global = true)]
    json: bool,

    #[arg(long, global = true, value_name = "DIR")]
    state_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Install a legacy .archive ZIP after explicit approval.
    Install {
        archive: PathBuf,
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long, help = "Confirm writing files into the game")]
        yes: bool,
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
    /// Preview where files would be installed; never writes to the game.
    Plan {
        archive: PathBuf,
        #[arg(long)]
        path: Option<PathBuf>,
    },
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
    Check { directory: PathBuf },
}

#[derive(Subcommand)]
enum GameCommand {
    /// Register a Cyberpunk installation by detection or explicit path.
    Add {
        game: String,
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Show registered installations.
    List,
}

fn database_path(state_dir: Option<&Path>) -> Result<PathBuf> {
    if let Some(dir) = state_dir {
        return Ok(dir.join("state.sqlite"));
    }
    if let Some(path) = std::env::var_os("LOCALAPPDATA") {
        return Ok(PathBuf::from(path).join("modn/state.sqlite"));
    }
    if let Some(path) = std::env::var_os("XDG_STATE_HOME") {
        return Ok(PathBuf::from(path).join("modn/state.sqlite"));
    }
    let home = std::env::var_os("HOME").context("no HOME or app state directory available")?;
    Ok(PathBuf::from(home).join(".local/state/modn/state.sqlite"))
}

fn validate_state_path(state_path: &Path) -> Result<()> {
    let parent = state_path.parent().context("invalid state path")?;
    let mut directory = if parent.is_absolute() {
        parent.to_path_buf()
    } else {
        std::env::current_dir()?.join(parent)
    };
    loop {
        if let Ok(existing) = directory.canonicalize() {
            if existing
                .ancestors()
                .any(|ancestor| ancestor.join("bin/x64/Cyberpunk2077.exe").is_file())
            {
                bail!("state directory must be outside the Cyberpunk 2077 installation");
            }
            return Ok(());
        }
        directory = directory
            .parent()
            .context("cannot resolve state directory")?
            .to_path_buf();
    }
}

fn choose_candidate(path: Option<PathBuf>) -> Result<Candidate> {
    if let Some(path) = path {
        return Ok(Candidate {
            path: validate_installation(&path)?,
            source: "manual".into(),
        });
    }
    let candidates = discover();
    match candidates.as_slice() {
        [candidate] => Ok(candidate.clone()),
        [] => bail!("Cyberpunk 2077 not found; specify --path to its installation directory"),
        _ => bail!("multiple installations found; specify --path to select one"),
    }
}

fn selected_installation(store: &Store, path: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = path {
        return validate_installation(&path);
    }
    let entries = store.list()?;
    match entries.as_slice() {
        [entry] => validate_installation(&entry.path),
        [] => bail!("no game registered; run 'modn game add cyberpunk --path <directory>'"),
        _ => bail!("multiple games registered; specify --path to select an installation"),
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let state_path = database_path(cli.state_dir.as_deref())?;
    let mut store = if matches!(
        &cli.command,
        Command::Inspect { .. } | Command::Module { .. }
    ) {
        None
    } else {
        validate_state_path(&state_path)?;
        Some(Store::open(&state_path)?)
    };
    match cli.command {
        Command::Module {
            command: ModuleCommand::Check { directory },
        } => {
            let module = check_module(&directory)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&module)?);
            } else {
                println!(
                    "Game module {} v{} (host API {})",
                    module.game_id, module.version, module.api_version
                );
            }
        }
        Command::Install { archive, path, yes } => {
            if !yes {
                bail!(
                    "installation writes game files; rerun with --yes after reviewing 'modn plan'"
                );
            }
            let game = selected_installation(
                store.as_ref().expect("non-inspect command has store"),
                path,
            )?;
            let package = store
                .as_mut()
                .expect("non-inspect command has store")
                .install_zip(&game, &archive)?;
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
        Command::Installed { path } => {
            let game = selected_installation(
                store.as_ref().expect("non-inspect command has store"),
                path,
            )?;
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
            let game = selected_installation(
                store.as_ref().expect("non-inspect command has store"),
                path,
            )?;
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
        Command::Plan { archive, path } => {
            let inspection = inspect(&archive)?;
            let game = selected_installation(
                store.as_ref().expect("non-inspect command has store"),
                path,
            )?;
            let report = plan(&inspection, &game);
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "Plan: {} (metadata only; {})",
                    report.status,
                    archive.display()
                );
                for file in report.operations {
                    println!("{} => {}", file.source, file.destination);
                }
                for issue in report.blockers {
                    println!("BLOCKER {}: {}", issue.code, issue.message);
                }
            }
        }
        Command::Game {
            command: GameCommand::Add { game, path },
        } => {
            if !["cyberpunk", "cyberpunk-2077"].contains(&game.as_str()) {
                bail!("unsupported game '{game}'; only cyberpunk-2077 is available in v0");
            }
            let candidate = choose_candidate(path)?;
            let entry = store
                .as_ref()
                .expect("non-inspect command has store")
                .add_cyberpunk(&candidate.path, &candidate.source)?;
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
                .expect("non-inspect command has store")
                .list()?;
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
            let path = selected_installation(
                store.as_ref().expect("non-inspect command has store"),
                path,
            )?;
            let report = scan_with_ownership(
                store.as_ref().expect("non-inspect command has store"),
                &path,
            )?;
            if report.incomplete {
                eprintln!("warning: modn scan incomplete (entry limit reached)");
            }
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("Cyberpunk 2077: {}", report.installation.display());
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
            let path = selected_installation(
                store.as_ref().expect("non-inspect command has store"),
                path,
            )?;
            let report = doctor_with_ownership(
                store.as_ref().expect("non-inspect command has store"),
                &path,
            )?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("Cyberpunk 2077: {}", report.scan.installation.display());
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
