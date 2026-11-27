use super::{open_installation, print_json, PriorityArgs};
use crate::state_access::InstallationAccess;
use anyhow::{Context, Result};
use modn_core::error::{coded, ErrorCode};
use modn_core::manager_roots::ManagerRoots;
use modn_core::profiles::{self, Placement};
use modn_core::state::Store;
use modn_core::transactions::{PriorityReport, ToggleChange};
use serde_json::json;
use std::path::Path;

pub(super) fn run(
    args: &PriorityArgs,
    roots: &ManagerRoots,
    game: Option<&str>,
    json: bool,
) -> Result<()> {
    let access = if args.dry_run {
        InstallationAccess::Preview
    } else {
        InstallationAccess::Write
    };
    let selected = open_installation(roots, args.path.clone(), game, access)?;
    let mut store = selected
        .store
        .expect("the selected installation has manager state");
    let profile = match &args.profile {
        Some(profile) => profile.clone(),
        None => active_profile(&store, &selected.root)?,
    };
    let placement = placement(args);
    if args.dry_run {
        let report =
            store.plan_priority(&selected.root, &profile, &args.package, &placement, true)?;
        return print_report(&report, json);
    }
    if !args.apply {
        let report = store.move_priority(&selected.root, &profile, &args.package, &placement)?;
        return print_report(&report, json);
    }
    let planned =
        store.plan_priority(&selected.root, &profile, &args.package, &placement, false)?;
    if !planned.changes.is_empty() && !args.yes {
        return Err(coded(
            ErrorCode::ApprovalRequired,
            format!(
                "profile {profile:?} changes game files; rerun with --yes, or --dry-run to preview"
            ),
        ));
    }
    let report =
        store.set_priority_applied(&selected.root, &profile, &args.package, &placement, true)?;
    print_report(&report, json)
}

fn placement(args: &PriorityArgs) -> Placement {
    if let Some(rank) = args.rank {
        return Placement::Rank(i64::from(rank));
    }
    if let Some(after) = &args.after {
        return Placement::After(after.clone());
    }
    Placement::Before(args.before.clone().unwrap_or_default())
}

fn active_profile(store: &Store, game: &Path) -> Result<String> {
    profiles::list(store, game)?
        .into_iter()
        .find(|profile| profile.active)
        .map(|profile| profile.name)
        .context("the installation has no active profile")
}

fn print_report(report: &PriorityReport, json: bool) -> Result<()> {
    let changes: Vec<_> = report.changes.iter().map(change_json).collect();
    if json {
        return print_json(&json!({
            "profile": report.profile,
            "package": report.package,
            "rank": report.rank,
            "applied": report.applied,
            "changes": changes,
            "dry_run": report.dry_run,
        }));
    }
    if report.dry_run {
        print_changes(&report.changes, "Would change");
        println!(
            "{} would move to rank {} in profile {:?}; --dry-run wrote nothing.",
            report.package, report.rank, report.profile
        );
        return Ok(());
    }
    if !report.applied {
        println!(
            "{} is now rank {} in profile {:?}. Game files still follow the previous priority order; run 'modn profile apply --yes' or repeat with --apply.",
            report.package, report.rank, report.profile
        );
        return Ok(());
    }
    if report.changes.is_empty() {
        println!(
            "{} is now rank {} in profile {:?}; game files already follow the new order.",
            report.package, report.rank, report.profile
        );
    } else {
        print_changes(&report.changes, "Changed");
    }
    Ok(())
}

fn change_json(change: &ToggleChange) -> serde_json::Value {
    json!({
        "path": change.path,
        "action": change.action.as_str(),
        "from": change.from,
        "to": change.to,
    })
}

fn print_changes(changes: &[ToggleChange], prefix: &str) {
    println!("{prefix} {} file(s):", changes.len());
    for change in changes {
        match (&change.from, &change.to) {
            (Some(from), Some(to)) => {
                println!(
                    "  {} {} with {to} (was {from})",
                    change.action.as_str(),
                    change.path
                );
            }
            (Some(from), None) => println!("  remove {} (was {from})", change.path),
            (None, Some(to)) => println!("  create {} with {to}", change.path),
            (None, None) => println!("  {} {}", change.action.as_str(), change.path),
        }
    }
}
