use crate::catalog_install::reference_of;
use anyhow::Result;
use modn_core::buckets::{BucketPackage, BucketStore, VersionReq};
use modn_core::planning::InstallPlan;
use modn_core::state::Store;
use modn_core::transactions::{
    CatalogRelease, DependencyState, ManagedPackage, RecordedRequirement,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::Path;

pub(super) struct FileChanges {
    added: Vec<String>,
    removed: Vec<String>,
    replaced: Vec<String>,
}

impl FileChanges {
    pub(super) fn new(old: &ManagedPackage, plan: &InstallPlan) -> Self {
        let key = |path: &str| path.to_lowercase();
        let old_paths: BTreeSet<String> = old.files.iter().map(|f| key(&f.destination)).collect();
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
        for file in &old.files {
            if !new_paths.contains(&key(&file.destination)) {
                changes.removed.push(file.destination.clone());
            }
        }
        changes
    }

    fn as_json(&self) -> Value {
        json!({"added": self.added, "removed": self.removed, "replaced": self.replaced})
    }
}

pub(super) struct DependentCheck {
    package_id: String,
    package: String,
    requirement: Requirement,
    satisfied: bool,
}

enum Requirement {
    Unknown,
    Unrestricted,
    Requires(String),
}

impl DependentCheck {
    pub(super) fn package_id_label(&self) -> String {
        format!("{} {}", self.package_id, self.package)
    }
}

fn installed_requirement(
    store: &Store,
    catalog: &BucketStore,
    dependent: &ManagedPackage,
    target: &BucketPackage,
) -> Result<Requirement> {
    let Some(reference) = &dependent.catalog else {
        return Ok(Requirement::Unknown);
    };
    let dependency = reference_of(target);
    Ok(
        match store.recorded_requirement(&dependent.id, &dependency)? {
            RecordedRequirement::Requires(text) => Requirement::Requires(text),
            RecordedRequirement::Unrestricted => Requirement::Unrestricted,
            _ => catalog
                .releases(&reference.bucket, &reference.game_id, &reference.mod_id)
                .ok()
                .and_then(|releases| {
                    releases
                        .into_iter()
                        .find(|release| release.package.sha256 == dependent.archive_sha256)
                })
                .map_or(Requirement::Unknown, |release| {
                    release
                        .package
                        .dependency_versions
                        .get(&target.package.id)
                        .cloned()
                        .map_or(Requirement::Unrestricted, Requirement::Requires)
                }),
        },
    )
}

pub(super) fn dependent_checks(
    store: &Store,
    game: &Path,
    package_id: &str,
    catalog: &BucketStore,
    target: &BucketPackage,
) -> Result<Vec<DependentCheck>> {
    store
        .dependents_of(game, package_id)?
        .into_iter()
        .map(|dependent| {
            let requirement = installed_requirement(store, catalog, &dependent, target)?;
            let satisfied = match &requirement {
                Requirement::Requires(text) => VersionReq::parse(text)?
                    .matches(&target.package.version)
                    .unwrap_or(false),
                Requirement::Unrestricted => true,
                Requirement::Unknown => false,
            };
            Ok(DependentCheck {
                package: dependent
                    .catalog
                    .as_ref()
                    .map_or_else(String::new, ToString::to_string),
                package_id: dependent.id,
                requirement,
                satisfied,
            })
        })
        .collect()
}

pub(super) fn dependency_blockers(
    release: &CatalogRelease,
    states: &[DependencyState],
) -> Vec<String> {
    release
        .dependencies
        .iter()
        .zip(states)
        .filter_map(|(dependency, state)| {
            let problem = match state {
                DependencyState::Ready { .. } => return None,
                DependencyState::Damaged { .. } => "is damaged",
                DependencyState::OtherRelease { .. } => "is another release",
                DependencyState::Missing => "is not installed",
                _ => "is in an unknown state",
            };
            Some(format!(
                "dependency {} {problem}; install, restore or update it first",
                dependency.reference
            ))
        })
        .collect()
}

pub(super) fn dependent_blockers(checks: &[DependentCheck], mod_id: &str) -> Vec<String> {
    checks
        .iter()
        .filter(|check| !check.satisfied)
        .map(|check| match &check.requirement {
            Requirement::Requires(text) => format!(
                "{} {} requires {mod_id} {text}",
                check.package_id, check.package
            ),
            _ => format!(
                "{} {}: the catalog no longer has its installed release, so its requirement on {mod_id} is unknown; update or reinstall it first",
                check.package_id, check.package
            ),
        })
        .collect()
}

pub(super) struct Preview<'a> {
    pub(super) reference: &'a str,
    pub(super) old: &'a ManagedPackage,
    pub(super) installed_version: Option<&'a str>,
    pub(super) target: &'a BucketPackage,
    pub(super) files: Option<FileChanges>,
    pub(super) plan: Option<&'a InstallPlan>,
    pub(super) archive_needed: Option<String>,
    pub(super) dependents: &'a [DependentCheck],
    pub(super) blockers: &'a [String],
    pub(super) unapproved_declared_executables: &'a [String],
}

impl Preview<'_> {
    pub(super) fn up_to_date(&self) -> bool {
        self.old.archive_sha256 == self.target.package.sha256
    }

    pub(super) fn print(&self, json: bool) -> Result<()> {
        if json {
            println!("{}", serde_json::to_string_pretty(&self.as_json())?);
            return Ok(());
        }
        let from = self.installed_version.unwrap_or("an unlisted release");
        println!(
            "{}: installed {from} ({}), target {}",
            self.reference, self.old.id, self.target.package.version
        );
        if self.up_to_date() {
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
                "package_id": self.old.id,
                "version": self.installed_version,
                "archive_sha256": self.old.archive_sha256,
                "exclusive_folders": self.old.directories,
            },
            "target": {
                "version": self.target.package.version,
                "archive_sha256": self.target.package.sha256,
                "source": self.target.package.source,
            },
            "up_to_date": self.up_to_date(),
            "files": self.files.as_ref().map(FileChanges::as_json),
            "archive_needed": self.archive_needed,
            "dependents": self.dependents.iter().map(|check| json!({
                "package_id": check.package_id,
                "package": check.package,
                "requirement": match &check.requirement {
                    Requirement::Requires(text) => Some(text.as_str()),
                    _ => None,
                },
                "requirement_known": !matches!(check.requirement, Requirement::Unknown),
                "satisfied": check.satisfied,
            })).collect::<Vec<_>>(),
            "blockers": self.blockers,
            "plan": self.plan,
            "unapproved_declared_executables": self.unapproved_declared_executables,
        })
    }
}
