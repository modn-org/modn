use crate::nxm::NxmLink;
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "modn", about = "Game mod inspection and guarded management")]
pub(super) struct Cli {
    #[arg(long, global = true)]
    pub(super) json: bool,

    /// Stream operation events as JSON lines on stderr; stdin line `cancel` cancels.
    #[arg(long, global = true, value_name = "FORMAT")]
    pub(super) progress: Option<crate::progress::ProgressFormat>,

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
    /// Claim matching manual files without changing game bytes.
    Adopt(AdoptArgs),
    /// Refresh every catalog, or replace an installed catalog mod with another release.
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
    /// Inspect the local official bucket and registered community catalogs.
    Bucket {
        #[command(subcommand)]
        command: BucketCommand,
    },
    /// List manager-owned packages (unmanaged mods are never included).
    List {
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Remove a managed package; --force removes changed owned files only.
    Uninstall(UninstallArgs),
    /// List committed install, update and uninstall operations, newest first.
    History,
    /// Undo one committed operation, or the newest one.
    Undo(UndoArgs),
    /// Safely roll back interrupted installs/uninstalls.
    Recover {
        #[arg(long, help = "Confirm rollback of interrupted transactions")]
        yes: bool,
    },
    /// Inspect ZIP, JAR, or 7z metadata without extracting files.
    Inspect { archive: PathBuf },
    /// Inspect asset structures without selecting a game or opening manager state.
    Asset {
        #[command(subcommand)]
        command: AssetCommand,
    },
    /// Check local Lua policy or manage explicitly trusted module snapshots.
    Module {
        #[command(subcommand)]
        command: ModuleCommand,
    },
    /// Manage credentials for remote mod sources.
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    Game {
        #[command(subcommand)]
        command: GameCommand,
    },
    /// Launch a registered game installation with its module's launch target.
    Play {
        /// Game ID to launch.
        game: String,
        /// Select one installation when several are registered.
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Inspect known mod directories without modifying game files.
    Scan {
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Run the selected game module's diagnostics and report managed-file drift.
    Doctor {
        #[arg(long)]
        path: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub(super) enum AssetCommand {
    /// Inspect DDS or RDAR v12 structural metadata without decoding payloads.
    Inspect { path: PathBuf },
}

#[derive(Subcommand)]
pub(super) enum ModuleCommand {
    /// Copy executable Lua policy into the runtime registry after explicit trust.
    Register {
        directory: PathBuf,
        #[arg(long, help = "Accept execution of this third-party Lua policy")]
        trust: bool,
    },
    /// List retained releases and verify their bytes.
    List,
    /// Remove a snapshot without changing installed packages or game files.
    Remove { game: String },
    /// Keep the active release and refuse automatic updates and rollback.
    Pin { game: String },
    /// Allow updates and rollback of the module again.
    Unpin { game: String },
    /// Fetch the source bucket and switch to a newer retained release.
    Update { game: String },
    /// Switch the active release pointer to an older retained release.
    Rollback {
        game: String,
        #[arg(long, value_name = "VERSION")]
        to: Option<String>,
    },
    /// Load a local game module and report its identity and API compatibility.
    Check {
        directory: PathBuf,
        /// Explicitly permit read-only discovery and inspection within this directory.
        #[arg(long = "allow-root", value_name = "DIR")]
        allow_roots: Vec<PathBuf>,
    },
}

#[derive(Subcommand)]
pub(super) enum AuthCommand {
    /// Store, clear or report the Nexus Mods API key.
    Nexus {
        #[command(subcommand)]
        command: NexusAuthCommand,
    },
}

#[derive(Subcommand)]
pub(super) enum NexusAuthCommand {
    /// Read a Nexus API key from stdin and save it in the OS credential store.
    Set,
    /// Remove the saved Nexus API key from the OS credential store.
    Clear,
    /// Report whether a Nexus API key is available.
    Status,
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
    /// Find installations in the Steam, GOG and Epic libraries.
    Detect {
        /// Register each new confirmed installation (writes manager state only).
        #[arg(long)]
        register: bool,
    },
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
    /// Explicitly fetch and validate a community Git catalog snapshot.
    Sync {
        name: String,
        #[arg(long, requires = "revision")]
        source: Option<String>,
        #[arg(long, requires = "source")]
        revision: Option<String>,
    },
    /// Download, verify and replace the official main bucket.
    Update { name: Option<String> },
    /// Register a local bucket directory after validating its complete contents.
    Add { name: String, path: PathBuf },
    /// Forget a local bucket registration without changing its files.
    Remove { name: String },
    /// Trust modules from a signed Git catalog and import its releases.
    TrustModules {
        bucket: String,
        #[arg(long, help = "Accept execution of this bucket's Lua modules")]
        yes: bool,
        #[arg(long, value_name = "KEY_ID")]
        fingerprint: Option<String>,
    },
    /// Stop trusting a bucket's modules; retained releases are refused at use.
    UntrustModules { bucket: String },
    /// List the local official bucket and registered community buckets.
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
    /// Nexus "Mod Manager Download" link for a free Nexus account. The link
    /// expires quickly and may remain in your shell history.
    #[arg(long, value_name = "LINK")]
    pub(super) nxm: Vec<String>,
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
    /// Manage a package whose manifest marks it as a native injector.
    #[arg(long)]
    pub(super) allow_native_injector: bool,
    #[arg(skip)]
    pub(super) nxm_links: Vec<NxmLink>,
}

/// Options for reversing one committed operation.
#[derive(Args)]
pub(super) struct UndoArgs {
    /// Operation ID from `modn history`; omit it to undo the newest operation.
    pub(super) id: Option<String>,
    #[arg(long)]
    pub(super) path: Option<PathBuf>,
    #[arg(
        long,
        help = "Confirm changing game files back",
        conflicts_with = "dry_run"
    )]
    pub(super) yes: bool,
    #[arg(
        long,
        help = "Show the inverse operation and its packages without writing",
        conflicts_with = "yes"
    )]
    pub(super) dry_run: bool,
    /// Independently authorize one exact game-relative tool executable in a restored archive.
    #[arg(long = "allow-executable", value_name = "GAME_RELATIVE_PATH")]
    pub(super) allow_executable: Vec<String>,
    /// Restore a package whose manifest marks it as a native injector.
    #[arg(long)]
    pub(super) allow_native_injector: bool,
}

/// Options for refreshing every catalog or replacing an installed catalog mod.
#[derive(Args)]
pub(super) struct UpdateArgs {
    /// Catalog name of the installed mod: `mod`, `game/mod` or `bucket/game/mod`.
    /// Omit it to refresh the official and every registered Git catalog.
    pub(super) target: Option<String>,
    /// Release to install; defaults to the bucket's current release.
    #[arg(long)]
    pub(super) version: Option<String>,
    #[arg(long)]
    pub(super) path: Option<PathBuf>,
    /// Manually acquired archive of the target release.
    #[arg(long, value_name = "ARCHIVE")]
    pub(super) file: Option<PathBuf>,
    /// Nexus "Mod Manager Download" link for a free Nexus account. The link
    /// expires quickly and may remain in your shell history.
    #[arg(long, value_name = "LINK")]
    pub(super) nxm: Vec<String>,
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
    /// Manage a package whose manifest marks it as a native injector.
    #[arg(long)]
    pub(super) allow_native_injector: bool,
    #[arg(skip)]
    pub(super) nxm_links: Vec<NxmLink>,
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
    /// Remove recorded files even when modified; requires --yes to execute.
    #[arg(long, conflicts_with_all = ["include_deps", "cascade"])]
    pub(super) force: bool,
}

/// Explicit approval to claim individual manual files from a local archive.
#[derive(Args)]
pub(super) struct AdoptArgs {
    pub(super) archive: PathBuf,
    #[arg(long)]
    pub(super) path: Option<PathBuf>,
    /// Preview metadata only; byte matching and ownership checks remain pending.
    #[arg(long, conflicts_with_all = ["yes", "acknowledge"])]
    pub(super) dry_run: bool,
    /// Acknowledge recording ownership of matching existing individual files.
    #[arg(long)]
    pub(super) acknowledge: bool,
    /// Confirm the atomic ownership commit; game bytes remain unchanged.
    #[arg(long)]
    pub(super) yes: bool,
}
