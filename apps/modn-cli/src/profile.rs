//! `modn profile`: named profiles and the priority of their packages.

mod apply;
mod enable;
mod lifecycle;
mod list;
mod priority;
mod switch;

use crate::state_access::{InstallationAccess, ManagerState, SelectedState};
use anyhow::Result;
use clap::{ArgGroup, Args, Subcommand};
use modn_core::error::{coded, ErrorCode};
use modn_core::manager_roots::ManagerRoots;
use modn_core::profiles::Profile;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Subcommand)]
pub(crate) enum ProfileCommand {
    /// List stored profiles and mark the active one.
    List(ListArgs),
    /// Create a profile, optionally from a copy of another profile.
    Create(CreateArgs),
    /// Delete a profile and its recorded priorities.
    Delete(DeleteArgs),
    /// Place a package before or after another package, or at one rank.
    Priority(PriorityArgs),
    /// Enable a package in a profile.
    Enable(EnableArgs),
    /// Disable a package in a profile.
    Disable(EnableArgs),
    /// Switch the active profile after comparing both file sets.
    Switch(SwitchArgs),
    /// Apply the selected profile's file priorities to the game.
    Apply(ApplyArgs),
}

/// Options of `modn profile list`.
#[derive(Args)]
pub(crate) struct ListArgs {
    /// Game installation directory; defaults to the selected installation.
    #[arg(long, value_name = "DIR")]
    pub(crate) path: Option<PathBuf>,
}

/// Options of `modn profile create`.
#[derive(Args)]
pub(crate) struct CreateArgs {
    /// Name of the new profile.
    pub(crate) name: String,
    /// Copy the packages and priorities of this profile.
    #[arg(long, value_name = "NAME")]
    pub(crate) clone: Option<String>,
    /// Game installation directory; defaults to the selected installation.
    #[arg(long, value_name = "DIR")]
    pub(crate) path: Option<PathBuf>,
}

/// Options of `modn profile delete`.
#[derive(Args)]
pub(crate) struct DeleteArgs {
    /// Name of the profile to delete.
    pub(crate) name: String,
    /// Confirm deleting the profile.
    #[arg(long)]
    pub(crate) yes: bool,
    /// Game installation directory; defaults to the selected installation.
    #[arg(long, value_name = "DIR")]
    pub(crate) path: Option<PathBuf>,
}

/// Options of `modn profile priority`.
#[derive(Args)]
#[command(group(
    ArgGroup::new("placement")
        .required(true)
        .args(["before", "after", "rank"])
))]
pub(crate) struct PriorityArgs {
    /// Managed package to move.
    pub(crate) package: String,
    /// Place the package immediately before this package.
    #[arg(long, value_name = "PACKAGE")]
    pub(crate) before: Option<String>,
    /// Place the package immediately after this package.
    #[arg(long, value_name = "PACKAGE")]
    pub(crate) after: Option<String>,
    /// Place the package at this one-based rank.
    #[arg(long, value_name = "N")]
    pub(crate) rank: Option<u32>,
    /// Profile to change; defaults to the active profile.
    #[arg(long, value_name = "NAME")]
    pub(crate) profile: Option<String>,
    /// Game installation directory; defaults to the selected installation.
    #[arg(long, value_name = "DIR")]
    pub(crate) path: Option<PathBuf>,
}

/// Options of `modn profile enable` and `modn profile disable`.
#[derive(Args)]
pub(crate) struct EnableArgs {
    /// Managed package to change.
    pub(crate) package: String,
    /// Profile to change; defaults to the active profile.
    #[arg(long, value_name = "NAME")]
    pub(crate) profile: Option<String>,
    /// Game installation directory; defaults to the selected installation.
    #[arg(long, value_name = "DIR")]
    pub(crate) path: Option<PathBuf>,
}

/// Options of `modn profile switch`.
#[derive(Args)]
pub(crate) struct SwitchArgs {
    /// Name of the profile to activate.
    pub(crate) name: String,
    /// Show the file changes without writing game files or manager state.
    #[arg(long)]
    pub(crate) dry_run: bool,
    /// Confirm changing game files.
    #[arg(long)]
    pub(crate) yes: bool,
    /// Game installation directory; defaults to the selected installation.
    #[arg(long, value_name = "DIR")]
    pub(crate) path: Option<PathBuf>,
}

/// Options of `modn profile apply`.
#[derive(Args)]
pub(crate) struct ApplyArgs {
    /// Profile to apply; defaults to the active profile.
    #[arg(long, value_name = "NAME")]
    pub(crate) profile: Option<String>,
    /// Confirm changing game files.
    #[arg(long)]
    pub(crate) yes: bool,
    /// Game installation directory; defaults to the selected installation.
    #[arg(long, value_name = "DIR")]
    pub(crate) path: Option<PathBuf>,
}

pub(crate) fn run(
    command: &ProfileCommand,
    roots: &ManagerRoots,
    game: Option<&str>,
    json: bool,
) -> Result<()> {
    match command {
        ProfileCommand::List(args) => list::run(args, roots, game, json),
        ProfileCommand::Create(args) => lifecycle::create(args, roots, game, json),
        ProfileCommand::Delete(args) => lifecycle::delete(args, roots, game, json),
        ProfileCommand::Priority(args) => priority::run(args, roots, game, json),
        ProfileCommand::Enable(args) => enable::enable(args, roots, game, json),
        ProfileCommand::Disable(args) => enable::disable(args, roots, game, json),
        ProfileCommand::Switch(args) => switch::run(args, roots, game, json),
        ProfileCommand::Apply(args) => apply::run(args, roots, game, json),
    }
}

/// The placeholder error of a profile command that is not implemented yet.
pub(super) fn not_implemented(command: &str) -> anyhow::Error {
    coded(
        ErrorCode::ProfileFailed,
        format!("not implemented yet: {command}"),
    )
}

/// Opens the selected game installation through the shared state access path.
///
/// `access` decides whether the command may write manager state. The writable
/// path is the same store opener that other commands use, including its schema
/// validation.
pub(super) fn open_installation(
    roots: &ManagerRoots,
    path: Option<PathBuf>,
    game: Option<&str>,
    access: InstallationAccess,
) -> Result<SelectedState> {
    ManagerState::load(roots)?
        .for_installation(roots, path, game)?
        .open(access)
}

/// One package in a profile, in rank order.
#[derive(Serialize)]
pub(super) struct PackageView<'a> {
    /// Managed package identifier.
    pub(super) package: &'a str,
    /// One-based position in priority order; rank 1 wins.
    pub(super) rank: usize,
    /// Stored priority; a higher value wins.
    pub(super) priority: i64,
    /// Whether the profile considers the package.
    pub(super) enabled: bool,
    /// Claimed files that another member wins.
    pub(super) shadowed_files: usize,
}

/// One profile as `--json` prints it.
#[derive(Serialize)]
pub(super) struct ProfileView<'a> {
    /// User-facing profile name.
    pub(super) name: &'a str,
    /// Whether this profile decides the files on disk.
    pub(super) active: bool,
    /// Stored creation time, as SQLite wrote it.
    pub(super) created_at: &'a str,
    /// Memberships in rank order.
    pub(super) packages: Vec<PackageView<'a>>,
}

impl<'a> ProfileView<'a> {
    pub(super) fn new(profile: &'a Profile) -> Self {
        Self {
            name: &profile.name,
            active: profile.active,
            created_at: &profile.created_at,
            packages: profile
                .packages
                .iter()
                .enumerate()
                .map(|(index, package)| PackageView {
                    package: &package.package_id,
                    rank: index + 1,
                    priority: package.priority,
                    enabled: package.enabled,
                    shadowed_files: package.shadowed_files,
                })
                .collect(),
        }
    }
}

/// Prints `value` as one pretty JSON document.
pub(super) fn print_json(value: &impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
