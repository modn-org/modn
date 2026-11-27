use crate::download;
use anyhow::Result;
use modn_core::acquisition::SourceAttempt;
use modn_core::buckets::BucketPackage;
use modn_core::catalog_pipeline::{
    CatalogPlan, DependentRequirement, InstalledPackage, PlanRefusal, PlannedDependent,
};
use modn_core::planning::InstallPlan;
use modn_core::transactions::DependencyState;
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub(super) struct FileChanges {
    added: Vec<String>,
    removed: Vec<String>,
    replaced: Vec<String>,
}

impl FileChanges {
    pub(super) fn new(owned: &[String], plan: &InstallPlan) -> Self {
        let key = |path: &str| path.to_lowercase();
        let old_paths: BTreeSet<String> = owned.iter().map(|path| key(path)).collect();
        let new_paths: BTreeSet<String> = plan
            .operations
            .iter()
            .map(|op| key(&op.destination))
            .collect();
        let mut changes = Self {
            added: Vec::new(),
            removed: Vec::new(),
            replaced: Vec::new(),
        };
        for operation in &plan.operations {
            if old_paths.contains(&key(&operation.destination)) {
                changes.replaced.push(operation.destination.clone());
            } else {
                changes.added.push(operation.destination.clone());
            }
        }
        for path in owned {
            if !new_paths.contains(&key(path)) {
                changes.removed.push(path.clone());
            }
        }
        changes
    }

    fn as_json(&self) -> Value {
        json!({"added": self.added, "removed": self.removed, "replaced": self.replaced})
    }
}

/// The dependent label that the update prompt names.
pub(super) fn dependent_label(dependent: &PlannedDependent) -> String {
    format!("{} {}", dependent.package_id, dependent.package)
}

/// The blocker lines that the plan reports before it may write.
///
/// Only the order, dependency and dependent refusals are blockers in the
/// update report; the caller reports the remaining refusals separately.
pub(super) fn blockers(plan: &CatalogPlan) -> Vec<String> {
    let mut blockers = Vec::new();
    for refusal in &plan.refusals {
        match refusal {
            PlanRefusal::UpdateOrder { reason } => blockers.push(reason.clone()),
            PlanRefusal::DependencyNotReady { reference, state } => {
                blockers.extend(dependency_blocker(reference, state));
            }
            PlanRefusal::DependentBlocks { dependent } => {
                blockers.push(dependent_blocker(dependent, &plan.selected.mod_id));
            }
            _ => {}
        }
    }
    blockers
}

fn dependency_blocker(
    reference: &modn_core::transactions::CatalogReference,
    state: &DependencyState,
) -> Option<String> {
    let problem = match state {
        DependencyState::Ready { .. } => return None,
        DependencyState::Damaged { .. } => "is damaged",
        DependencyState::OtherRelease { .. } => "is another release",
        DependencyState::Missing => "is not installed",
        _ => "is in an unknown state",
    };
    Some(format!(
        "dependency {reference} {problem}; install, restore or update it first"
    ))
}

fn dependent_blocker(dependent: &PlannedDependent, mod_id: &str) -> String {
    match &dependent.requirement {
        DependentRequirement::Requires(text) => format!(
            "{} {}{} requires {mod_id} {text}",
            dependent.package_id,
            dependent.package,
            if dependent.pinned { " (pinned)" } else { "" }
        ),
        DependentRequirement::Alias(alias) => format!(
            "{} {} needs this archive as {alias}, not as {mod_id}; Modn cannot show that it accepts the update, so update or reinstall it first",
            dependent.package_id, dependent.package
        ),
        _ => format!(
            "{} {}: its requirement on {mod_id} is unknown; update or reinstall it first",
            dependent.package_id, dependent.package
        ),
    }
}

pub(super) struct Preview<'a> {
    pub(super) reference: &'a str,
    pub(super) installed: &'a InstalledPackage,
    pub(super) installed_version: Option<&'a str>,
    pub(super) target: &'a BucketPackage,
    pub(super) up_to_date: bool,
    pub(super) files: Option<FileChanges>,
    pub(super) plan: Option<&'a InstallPlan>,
    pub(super) archive_needed: Option<String>,
    pub(super) dependents: &'a [PlannedDependent],
    pub(super) blockers: &'a [String],
    pub(super) downloads: &'a [SourceAttempt],
    pub(super) unapproved_declared_executables: &'a [String],
}

impl Preview<'_> {
    pub(super) fn print(&self, json: bool) -> Result<()> {
        if json {
            println!("{}", serde_json::to_string_pretty(&self.as_json())?);
            return Ok(());
        }
        let from = self.installed_version.unwrap_or("an unlisted release");
        println!(
            "{}: installed {from} ({}), target {}",
            self.reference, self.installed.id, self.target.package.version
        );
        for attempt in self.downloads {
            println!("DOWNLOAD {}: {}", attempt.source, attempt.describe());
        }
        if self.up_to_date {
            println!("Already installed; nothing to update.");
            return Ok(());
        }
        if let Some(files) = &self.files {
            for (label, paths) in [
                ("add", &files.added),
                ("replace", &files.replaced),
                ("remove", &files.removed),
            ] {
                for path in paths {
                    println!("  {label} {path}");
                }
            }
        }
        if let Some(needed) = &self.archive_needed {
            println!("Archive needed: {needed}");
        }
        for check in self.dependents {
            let status = if check.satisfied {
                "keeps working"
            } else {
                "blocks the update"
            };
            println!("Dependent {} {}: {status}", check.package_id, check.package);
        }
        for blocker in self.blockers {
            println!("BLOCKER {blocker}");
        }
        if let Some(plan) = self.plan {
            for issue in &plan.blockers {
                println!("BLOCKER {}: {}", issue.code, issue.message);
            }
        }
        Ok(())
    }

    fn as_json(&self) -> Value {
        json!({
            "package": self.reference,
            "installed": {
                "package_id": self.installed.id,
                "version": self.installed_version,
                "archive_blake3": self.installed.archive_blake3,
                "source": self.installed.source,
                "exclusive_folders": self.installed.directories,
            },
            "target": {
                "version": self.target.package.version,
                "archive_blake3": self.target.package.pin(),
                "source": self.target.package.primary_source().clone(),
            },
            "up_to_date": self.up_to_date,
            "files": self.files.as_ref().map(FileChanges::as_json),
            "archive_needed": self.archive_needed,
            "downloads": download::downloads_json(self.downloads),
            "dependents": self.dependents.iter().map(|check| json!({
                "package_id": check.package_id,
                "package": check.package,
                "requirement": match &check.requirement {
                    DependentRequirement::Requires(text) => Some(text.as_str()),
                    _ => None,
                },
                "requirement_known": matches!(
                    check.requirement,
                    DependentRequirement::Requires(_) | DependentRequirement::Unrestricted
                ),
                "alias": match &check.requirement {
                    DependentRequirement::Alias(alias) => Some(alias.as_str()),
                    _ => None,
                },
                "satisfied": check.satisfied,
            })).collect::<Vec<_>>(),
            "blockers": self.blockers,
            "plan": self.plan,
            "unapproved_declared_executables": self.unapproved_declared_executables,
        })
    }
}
