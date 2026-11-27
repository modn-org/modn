//! `modn conflicts`: overlapping file claims between managed packages.

use crate::state_access::{InstallationAccess, ManagerState};
use anyhow::Result;
use clap::Args;
use modn_core::conflicts::{ConflictReport, PathConflict};
use modn_core::manager_roots::ManagerRoots;
use serde::Serialize;
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

/// The `modn conflicts --json` document.
#[derive(Serialize)]
struct ConflictsDocument<'a> {
    game: &'a str,
    path: String,
    #[serde(flatten)]
    report: &'a ConflictReport,
}

pub(crate) fn run(
    args: &ConflictsArgs,
    roots: &ManagerRoots,
    game: Option<&str>,
    json: bool,
) -> Result<()> {
    let selected = ManagerState::load(roots)?
        .for_installation(roots, args.path.clone(), game)?
        .open(InstallationAccess::Preview)?;
    let store = selected
        .store
        .as_ref()
        .expect("preview access requires manager state");
    let report = modn_core::conflicts::conflicts(store, &selected.root, args.profile.as_deref())?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&ConflictsDocument {
                game: &selected.game_id,
                path: selected.root.to_string_lossy().into_owned(),
                report: &report,
            })?
        );
        return Ok(());
    }
    print_report(&report);
    Ok(())
}

fn print_report(report: &ConflictReport) {
    for conflict in &report.conflicts {
        print_conflict(conflict, report.active);
        println!();
    }
    println!("{} overlapping files", report.conflicts.len());
}

fn print_conflict(conflict: &PathConflict, active: bool) {
    println!("{}/{}", conflict.root, conflict.path);
    for claim in &conflict.claims {
        let marker = if claim.winner { "WINNER" } else { "      " };
        println!(
            "  {marker} #{} {} priority {} blake3 {} {} bytes",
            claim.rank,
            claim.package,
            claim.priority,
            short_digest(&claim.blake3),
            claim.bytes
        );
    }
    match (&conflict.winner, active) {
        (Some(winner), true) if conflict.consistent => println!("  on disk: {winner}"),
        (Some(winner), true) => println!("  on disk: {winner}; run `modn doctor`"),
        (None, true) => println!("  on disk: none; run `modn doctor`"),
        _ => println!("  on disk: none (profile is inactive)"),
    }
}

fn short_digest(digest: &str) -> String {
    digest.chars().take(8).collect()
}
