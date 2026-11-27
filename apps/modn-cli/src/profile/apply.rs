use super::{open_installation, print_json, ApplyArgs};
use crate::state_access::InstallationAccess;
use anyhow::Result;
use modn_core::error::{coded, ErrorCode};
use modn_core::manager_roots::ManagerRoots;
use modn_core::transactions::{ProfileApply, ToggleChange};
use serde_json::json;

pub(super) fn run(
    args: &ApplyArgs,
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
    let plan = store.plan_profile_apply(&selected.root, args.profile.as_deref())?;
    if !plan.active {
        return Err(coded(
            ErrorCode::ProfileFailed,
            format!(
                "PROFILE_FAILED: profile {:?} is not active; switch to it before applying",
                plan.profile
            ),
        ));
    }
    if args.dry_run {
        return print_report(&plan, json, true);
    }
    if !plan.changes.is_empty() && !args.yes {
        return Err(coded(
            ErrorCode::ApprovalRequired,
            format!(
                "profile {:?} changes game files; rerun with --yes, or --dry-run to preview",
                plan.profile
            ),
        ));
    }
    let applied = if plan.changes.is_empty() {
        plan
    } else {
        store.apply_profile(&selected.root, args.profile.as_deref())?
    };
    print_report(&applied, json, false)
}

fn print_report(report: &ProfileApply, json: bool, dry_run: bool) -> Result<()> {
    let changes: Vec<_> = report.changes.iter().map(change_json).collect();
    if json {
        return print_json(&json!({
            "profile": report.profile,
            "changes": changes,
            "dry_run": dry_run,
        }));
    }
    if report.changes.is_empty() {
        println!(
            "Profile {:?} is already consistent with the game files.",
            report.profile
        );
        return Ok(());
    }
    let prefix = if dry_run { "Would change" } else { "Changed" };
    println!("{prefix} {} file(s):", report.changes.len());
    for change in &report.changes {
        print_change(change);
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

fn print_change(change: &ToggleChange) {
    match (&change.from, &change.to) {
        (Some(from), Some(to)) => println!(
            "  {} {} with {to} (was {from})",
            change.action.as_str(),
            change.path
        ),
        (Some(from), None) => println!("  remove {} (was {from})", change.path),
        (None, Some(to)) => println!("  create {} with {to}", change.path),
        (None, None) => println!("  {} {}", change.action.as_str(), change.path),
    }
}
