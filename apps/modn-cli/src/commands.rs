use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "modn", about = "Game mod inspection and guarded management")]
pub(super) struct Cli {
    #[arg(long, global = true)]
    pub(super) json: bool,

    #[arg(long, global = true, value_name = "DIR")]
    pub(super) state_dir: Option<PathBuf>,

    /// Select a runtime-loaded game policy by ID.
    #[arg(long = "game", global = true, value_name = "ID")]
    pub(super) game_id: Option<String>,

    #[command(subcommand)]
    pub(super) command: Command,
}

#[derive(Subcommand)]
pub(super) enum Command {
    /// Preview or install a ZIP or 7z mod with the selected Lua policy after approval.
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
        /// Independently authorize one exact game-relative tool executable.
        #[arg(long = "allow-executable", value_name = "GAME_RELATIVE_PATH")]
        allow_executable: Vec<String>,
    },
    /// Inspect bundled and explicitly registered local package catalogs.
    Bucket {
        #[command(subcommand)]
        command: BucketCommand,
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
    /// Validate a local Lua module without granting file or game-write access.
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
pub(super) enum ModuleCommand {
    /// Load a local game module and report its identity and API compatibility.
    Check {
        directory: PathBuf,
        /// Explicitly permit read-only discovery and inspection within this directory.
        #[arg(long = "allow-root", value_name = "DIR")]
        allow_roots: Vec<PathBuf>,
    },
    /// Validate an unsigned local module ZIP without creating state.
    Inspect { archive: PathBuf },
    /// Install an unsigned local ZIP; activation requires a separate enable.
    Install {
        archive: PathBuf,
        #[arg(long, conflicts_with = "yes")]
        dry_run: bool,
        #[arg(long)]
        yes: bool,
    },
    /// Install and select a new version of an already enabled user module.
    Update {
        archive: PathBuf,
        #[arg(long)]
        yes: bool,
    },
    /// Explicitly select an installed version (including over a bundled module).
    Enable {
        game: String,
        version: String,
        #[arg(long)]
        yes: bool,
    },
    /// Disable user selection, retaining installed and rollback versions.
    Disable {
        game: String,
        #[arg(long)]
        yes: bool,
    },
    /// Swap the selected and previous installed versions.
    Rollback {
        game: String,
        #[arg(long)]
        yes: bool,
    },
    /// List and integrity-check installed user versions without creating state.
    List,
    /// Show user selection, rollback target and effective module source.
    Status { game: String },
}

#[derive(Subcommand)]
pub(super) enum GameCommand {
    /// Register an installation using its game's Lua module.
    Add {
        game: String,
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Show registered installations.
    List,
}

#[derive(Subcommand)]
pub(super) enum BucketCommand {
    /// Register a local bucket directory after validating its complete contents.
    Add { name: String, path: PathBuf },
    /// Forget a local bucket registration without changing its files.
    Remove { name: String },
    /// List the bundled official bucket and registered local buckets.
    List,
    /// Find validated package manifests; --game filters results.
    Search,
    /// Inspect one exact bucket/game/package reference.
    Show { package: String },
    /// List versioned Lua game-module metadata without executing it.
    Modules,
    /// Inspect one exact bucket/game/module-version reference.
    ModuleShow { module: String },
}
