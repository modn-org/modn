use crate::commands::AdoptArgs;
use crate::state_access::{InstallationAccess, ManagerState, ProtectedTarget};
use anyhow::{bail, Result};
use modn_core::planning::{self, PlanStatus};
use modn_core::transactions::AdoptionApproval;
use std::path::Path;

pub(super) fn run(
    args: &AdoptArgs,
    state_path: &Path,
    requested: Option<&str>,
    json: bool,
) -> Result<()> {
    if !args.dry_run && !(args.acknowledge && args.yes) {
        bail!("adoption records ownership; use --dry-run or both --acknowledge and --yes");
    }
    let selected = ManagerState::load(state_path)?
        .for_installation(args.path.clone(), requested)?
        .open(InstallationAccess::ArchivePreview)?;
    let plan = planning::preview_adoption(&selected.game_id, &selected.root, &args.archive)?;
    if args.dry_run {
        let status = if plan.status == PlanStatus::Blocked {
            "blocked"
        } else {
            "verification_required"
        };
        let pending = [
            "target byte size and SHA-256 match",
            "staged archive content",
            "ownership and recorded conflicts",
            "current paths and pending transactions",
        ];
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "dry_run": true,
                    "status": status,
                    "target_contents_verified": false,
                    "checks_pending_until_commit": pending,
                    "plan": plan,
                }))?
            );
        } else {
            println!("Adoption preview: {status} (metadata only)");
            println!("Archive SHA-256: {}", plan.archive_sha256);
            for file in &plan.operations {
                println!(
                    "{} => {} ({} bytes)",
                    file.source, file.destination, file.bytes
                );
            }
            for issue in &plan.blockers {
                println!("BLOCKER {}: {}", issue.code, issue.message);
            }
            println!("Pending until approved commit: {}.", pending.join(", "));
        }
        return Ok(());
    }
    if plan.status == PlanStatus::Blocked {
        bail!(
            "adoption proposal is blocked: {}",
            serde_json::to_string(&plan.blockers)?
        );
    }
    let game_id = selected.game_id;
    let game = selected.root;
    drop(selected.store);
    let mut store = ManagerState::load(state_path)?.open_writable(&ProtectedTarget {
        game: Some(game_id.clone()),
        root: Some(game.clone()),
    })?;
    let package = store.adopt_archive(
        &game_id,
        &game,
        &args.archive,
        &plan.archive_sha256,
        AdoptionApproval::Acknowledged,
    )?;
    if json {
        println!("{}", serde_json::to_string_pretty(&package)?);
    } else {
        println!(
            "Adopted {} ({} individual owned files; game bytes unchanged)",
            package.id,
            package.files.len()
        );
    }
    Ok(())
}
