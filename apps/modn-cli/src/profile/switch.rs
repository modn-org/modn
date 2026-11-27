use super::{open_installation, print_json, SwitchArgs};
use crate::progress::Progress;
use crate::state_access::InstallationAccess;
use anyhow::Result;
use modn_core::error::{coded, ErrorCode};
use modn_core::manager_roots::ManagerRoots;
use modn_core::transactions::{
    plan_profile_switch, profile_switch, PathAction, ProfileSwitchOptions, ProfileSwitchOutcome,
    ProfileSwitchPlan,
};
use serde_json::json;

pub(super) fn run(
    args: &SwitchArgs,
    roots: &ManagerRoots,
    game: Option<&str>,
    json: bool,
    progress: &Progress,
) -> Result<()> {
    progress.begin()?;
    let access = if args.dry_run {
        InstallationAccess::Preview
    } else {
        InstallationAccess::Write
    };
    let selected = open_installation(roots, args.path.clone(), game, access)?;
    let mut store = selected
        .store
        .expect("the selected installation has manager state");
    let plan = plan_profile_switch(&store, &selected.root, &args.name)?;
    if args.dry_run {
        return print_plan(&plan, json);
    }
    if progress.enabled() && !args.yes {
        return Err(coded(
            ErrorCode::ApprovalRequired,
            "profile switch writes game files; pass --yes with --progress json",
        ));
    }
    if !args.yes {
        return Err(coded(
            ErrorCode::ApprovalRequired,
            format!(
                "profile switch to {:?} changes game files; rerun with --yes, or --dry-run to preview",
                args.name
            ),
        ));
    }
    let outcome = profile_switch(
        &mut store,
        &selected.root,
        &args.name,
        ProfileSwitchOptions::new(progress.control().clone()),
    )?;
    print_outcome(&outcome, json)
}

fn print_plan(plan: &ProfileSwitchPlan, json: bool) -> Result<()> {
    if json {
        let mut document = serde_json::to_value(plan)?;
        document
            .as_object_mut()
            .expect("the plan serializes as a JSON object")
            .insert("dry_run".to_owned(), json!(true));
        return print_json(&document);
    }
    print_header(plan);
    print_paths(plan);
    Ok(())
}

fn print_outcome(outcome: &ProfileSwitchOutcome, json: bool) -> Result<()> {
    let plan = &outcome.plan;
    if json {
        return print_json(&json!({
            "from": plan.from,
            "to": plan.to,
            "dry_run": false,
            "removed": outcome.removed,
            "written": outcome.written,
            "unchanged": outcome.unchanged,
            "entering": plan.entering,
            "leaving": plan.leaving,
            "operation_id": outcome.transaction_id,
        }));
    }
    print_header(plan);
    print_paths(plan);
    println!("Switched to profile {:?}.", plan.to);
    Ok(())
}

fn print_header(plan: &ProfileSwitchPlan) {
    println!("Switching profile {:?} -> {:?}", plan.from, plan.to);
    println!(
        "  {} to remove, {} to write ({} bytes), {} unchanged",
        plan.remove.len(),
        plan.write.len(),
        plan.bytes_to_write,
        plan.unchanged
    );
    println!("  entering: {}", names(&plan.entering));
    println!("  leaving: {}", names(&plan.leaving));
}

fn print_paths(plan: &ProfileSwitchPlan) {
    for path in &plan.remove {
        println!("  remove {} ({})", path.path, path.root);
    }
    for path in &plan.write {
        match path.action {
            PathAction::Replace => match &path.replaces {
                Some(replaces) => {
                    println!("  replace {} ({}; was {replaces})", path.path, path.root)
                }
                None => println!("  replace {} ({})", path.path, path.root),
            },
            _ => println!("  write {} ({})", path.path, path.root),
        }
    }
}

fn names(packages: &[String]) -> String {
    if packages.is_empty() {
        "(none)".to_owned()
    } else {
        packages.join(", ")
    }
}
