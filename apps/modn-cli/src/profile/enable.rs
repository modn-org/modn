use super::{open_installation, print_json, EnableArgs};
use crate::state_access::InstallationAccess;
use anyhow::{Context, Result};
use modn_core::error::{coded, ErrorCode};
use modn_core::manager_roots::ManagerRoots;
use modn_core::profiles;
use modn_core::transactions::{ProfileToggle, ToggleAction};
use serde_json::json;

pub(super) fn enable(
    args: &EnableArgs,
    roots: &ManagerRoots,
    game: Option<&str>,
    json: bool,
) -> Result<()> {
    run(args, roots, game, json, true)
}

pub(super) fn disable(
    args: &EnableArgs,
    roots: &ManagerRoots,
    game: Option<&str>,
    json: bool,
) -> Result<()> {
    run(args, roots, game, json, false)
}

fn run(
    args: &EnableArgs,
    roots: &ManagerRoots,
    game: Option<&str>,
    json: bool,
    enabled: bool,
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
        None => {
            profiles::list(&store, &selected.root)?
                .into_iter()
                .find(|profile| profile.active)
                .context("the installation has no active profile")?
                .name
        }
    };
    let plan = store.plan_profile_toggle(&selected.root, &profile, &args.package, enabled)?;
    if args.dry_run {
        return report(&plan, json, true);
    }
    if plan.active && !plan.changes.is_empty() && !args.yes {
        return Err(coded(
            ErrorCode::ApprovalRequired,
            format!(
                "profile {profile:?} changes game files; rerun with --yes, or --dry-run to preview"
            ),
        ));
    }
    let result = store.profile_toggle(&selected.root, &profile, &args.package, enabled)?;
    report(&result, json, false)
}

fn report(toggle: &ProfileToggle, json: bool, dry_run: bool) -> Result<()> {
    let changes: Vec<_> = toggle
        .changes
        .iter()
        .map(|change| {
            json!({
                "path": change.path,
                "action": change.action.as_str(),
                "from": change.from,
                "to": change.to,
            })
        })
        .collect();
    if json {
        return print_json(&json!({
            "profile": toggle.profile,
            "package": toggle.package,
            "enabled": toggle.enabled,
            "active": toggle.active,
            "changes": changes,
            "dry_run": dry_run,
        }));
    }
    let verb = if toggle.enabled {
        "enabled"
    } else {
        "disabled"
    };
    if !toggle.active {
        println!(
            "{} is now {verb} in inactive profile {:?}; game files are unchanged.",
            toggle.package, toggle.profile
        );
        return Ok(());
    }
    if toggle.changes.is_empty() {
        println!(
            "{} is already {verb} in profile {:?}; no files changed.",
            toggle.package, toggle.profile
        );
        return Ok(());
    }
    let prefix = if dry_run { "Would change" } else { "Changed" };
    println!(
        "{prefix} {} file(s) for {} in profile {:?}:",
        toggle.changes.len(),
        toggle.package,
        toggle.profile
    );
    for change in &toggle.changes {
        match change.action {
            ToggleAction::Remove => match &change.from {
                Some(from) => println!("  remove {} (was {from})", change.path),
                None => println!("  remove {}", change.path),
            },
            ToggleAction::Restore | ToggleAction::Replace => match (&change.from, &change.to) {
                (Some(from), Some(to)) => {
                    println!(
                        "  {} {} with {to} (was {from})",
                        change.action.as_str(),
                        change.path
                    )
                }
                _ => println!("  {} {}", change.action.as_str(), change.path),
            },
            ToggleAction::Create => match &change.to {
                Some(to) => println!("  create {} with {to}", change.path),
                None => println!("  create {}", change.path),
            },
            _ => println!("  {} {}", change.action.as_str(), change.path),
        }
    }
    Ok(())
}
