use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "modn", about = "Game mod inspection and guarded management")]
pub(super) struct Cli {
    #[arg(long, global = true)]
    pub(super) json: bool,

    #[arg(long, global = true, value_name = "DIR")]
    pub(super) state_dir: Option<PathBuf>,

    /// Select an official or explicitly trusted Lua game policy by ID.
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
    /// Replace an installed catalog mod with another release, recoverably.
    Update(UpdateArgs),
    /// Prevent updates of a managed package, by ID or catalog name.
    Pin(PinArgs),
    /// Allow updates of a managed package again.
    Unpin(PinArgs),
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
    /// Remove an unchanged manager-owned package, by ID or catalog name.
    Uninstall(UninstallArgs),
    /// Safely roll back interrupted installs/uninstalls.
    Recover {
        #[arg(long, help = "Confirm rollback of interrupted transactions")]
        yes: bool,
    },
    /// Inspect ZIP, JAR, or 7z metadata without extracting files.
    Inspect { archive: PathBuf },
    /// Check local Lua policy or manage explicitly trusted module snapshots.
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
    /// Copy executable Lua policy into the runtime registry after explicit trust.
    Register {
        directory: PathBuf,
        #[arg(long, help = "Accept execution of this third-party Lua policy")]
        trust: bool,
    },
    /// List trusted snapshots and verify their retained bytes.
    List,
    /// Remove a snapshot without changing installed packages or game files.
    Remove { game: String },
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
    /// Download, verify and replace the official main bucket.
    Update { name: Option<String> },
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
    /// With --dry-run, download missing `https` archives into the verified
    /// cache, so the preview can list their files. Game files stay unchanged.
    #[arg(long)]
    pub(super) download: bool,
    /// Independently authorize one exact game-relative tool executable in the archive.
    #[arg(long = "allow-executable", value_name = "GAME_RELATIVE_PATH")]
    pub(super) allow_executable: Vec<String>,
}

/// Options for replacing an installed catalog mod with another release.
#[derive(Args)]
pub(super) struct UpdateArgs {
    /// Catalog name of the installed mod: `mod`, `game/mod` or `bucket/game/mod`.
    pub(super) target: String,
    /// Release to install; defaults to the bucket's current release.
    #[arg(long)]
    pub(super) version: Option<String>,
    #[arg(long)]
    pub(super) path: Option<PathBuf>,
    /// Manually acquired archive of the target release.
    #[arg(long, value_name = "ARCHIVE")]
    pub(super) file: Option<PathBuf>,
    #[arg(
        long,
        help = "Confirm replacing the installed release in the game",
        conflicts_with = "dry_run"
    )]
    pub(super) yes: bool,
    #[arg(
        long,
        help = "Show the installed and target releases and the files that change, without writing",
        conflicts_with = "yes"
    )]
    pub(super) dry_run: bool,
    /// Download `https` archives again instead of using verified cache entries.
    #[arg(long)]
    pub(super) no_cache: bool,
    /// With --dry-run, download the target archive into the verified cache.
    #[arg(long)]
    pub(super) download: bool,
    /// Independently authorize one exact game-relative tool executable in the archive.
    #[arg(long = "allow-executable", value_name = "GAME_RELATIVE_PATH")]
    pub(super) allow_executable: Vec<String>,
}

/// Options for changing a managed package's pin.
#[derive(Args)]
pub(super) struct PinArgs {
    /// Managed-package ID or catalog name: mod, game/mod or bucket/game/mod.
    pub(super) package: String,
    #[arg(long)]
    pub(super) path: Option<PathBuf>,
}

/// Options for removing manager-owned packages.
#[derive(Args)]
pub(super) struct UninstallArgs {
    /// Managed-package ID from `modn list`, or the catalog name it was installed
    /// under: `mod`, `game/mod` or `bucket/game/mod`.
    pub(super) package: String,
    #[arg(long)]
    pub(super) path: Option<PathBuf>,
    #[arg(
        long,
        help = "Confirm removing managed game files",
        conflicts_with = "dry_run"
    )]
    pub(super) yes: bool,
    #[arg(
        long,
        help = "List the packages that would be removed, in order, and change nothing",
        conflicts_with = "yes"
    )]
    pub(super) dry_run: bool,
    /// Also remove each dependency that no remaining package needs.
    #[arg(long = "include-deps")]
    pub(super) include_deps: bool,
    /// Also remove every package that needs the selected one, dependents first.
    #[arg(long)]
    pub(super) cascade: bool,
}
