use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "modn", about = "Game mod inspection and guarded management")]
pub(super) struct Cli {
    #[arg(long, global = true)]
    pub(super) json: bool,

    #[arg(long, global = true, value_name = "DIR")]
    pub(super) state_dir: Option<PathBuf>,

    /// Select a first-party Lua game policy by ID.
    #[arg(long = "game", global = true, value_name = "ID")]
    pub(super) game_id: Option<String>,

    /// Use one local module source directory for this command only (development).
    #[arg(long = "module-src", alias = "src", global = true, value_name = "DIR")]
    pub(super) module_source: Option<PathBuf>,

    #[command(subcommand)]
    pub(super) command: Command,
}

#[derive(Subcommand)]
pub(super) enum Command {
    /// Install a cataloged mod or an explicit local ZIP/7z archive.
    Install(InstallArgs),
    /// Search mod manifests across configured buckets.
    Search {
        /// Match mod ID or display name (omit to list all).
        query: Option<String>,
    },
    /// Show or remove verified archives in the download cache.
    Cache {
        #[command(subcommand)]
        command: CacheCommand,
    },
    /// Inspect bundled and explicitly registered local package catalogs.
    Bucket {
        #[command(subcommand)]
        command: BucketCommand,
    },
    /// List manager-owned packages (unmanaged mods are never included).
    List {
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
    /// Unregister an installation only when no managed mods or pending operations remain.
    Remove {
        game: String,
        #[arg(long)]
        path: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub(super) enum CacheCommand {
    /// List cached archives, optionally of one mod.
    Show {
        #[arg(value_name = "MOD")]
        mod_id: Option<String>,
    },
    /// Remove the cached archives of one mod, or all of them with --all.
    Rm {
        #[arg(value_name = "MOD")]
        mod_id: Option<String>,
        #[arg(long)]
        all: bool,
    },
}

#[derive(Subcommand)]
pub(super) enum BucketCommand {
    /// Register a local bucket directory after validating its complete contents.
    Add { name: String, path: PathBuf },
    /// Forget a local bucket registration without changing its files.
    Remove { name: String },
    /// List the bundled official bucket and registered local buckets.
    List,
    /// Inspect one exact bucket/game/package reference.
    Show { package: String },
}

/// Options shared by catalog and standalone archive installation.
#[derive(Args)]
pub(super) struct InstallArgs {
    /// Mod ID, qualified bucket reference, or archive path.
    pub(super) target: String,
    #[arg(long)]
    pub(super) path: Option<PathBuf>,
    /// Manually acquired archive for a cataloged mod or a missing dependency.
    /// Repeat for each archive; Modn matches each one to its release by SHA-256.
    #[arg(long, value_name = "ARCHIVE")]
    pub(super) file: Vec<PathBuf>,
    #[arg(
        long,
        help = "Confirm writing files into the game",
        conflicts_with = "dry_run"
    )]
    pub(super) yes: bool,
    #[arg(
        long,
        help = "Preview operations without writing game files",
        conflicts_with = "yes"
    )]
    pub(super) dry_run: bool,
    /// Download `https` archives again instead of using verified cache entries.
    #[arg(long)]
    pub(super) no_cache: bool,
    /// Independently authorize one exact game-relative tool executable in the archive.
    #[arg(long = "allow-executable", value_name = "GAME_RELATIVE_PATH")]
    pub(super) allow_executable: Vec<String>,
}
