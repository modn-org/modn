//! `modn conflicts`: overlapping file claims between managed packages.

use anyhow::Result;
use clap::Args;
use modn_core::error::{coded, ErrorCode};
use modn_core::manager_roots::ManagerRoots;
use std::path::PathBuf;

/// Options of `modn conflicts`.
#[derive(Args)]
pub(crate) struct ConflictsArgs {
    /// Profile to inspect; defaults to the active profile.
    #[arg(long, value_name = "NAME")]
    pub(crate) profile: Option<String>,
    /// Game installation directory; defaults to the selected installation.
    #[arg(long, value_name = "DIR")]
    pub(crate) path: Option<PathBuf>,
}

pub(crate) fn run(
    _args: &ConflictsArgs,
    _roots: &ManagerRoots,
    _game: Option<&str>,
    _json: bool,
) -> Result<()> {
    Err(coded(
        ErrorCode::ConflictsFailed,
        "not implemented yet: conflicts",
    ))
}
