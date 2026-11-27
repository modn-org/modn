//! `modn profile`: named profiles and the priority of their packages.

mod apply;
mod enable;
mod lifecycle;
mod list;
mod priority;
mod switch;

use anyhow::Result;
use clap::{ArgGroup, Args, Subcommand};
use modn_core::error::{coded, ErrorCode};
use modn_core::manager_roots::ManagerRoots;
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
