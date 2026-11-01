//! Read-only dependency plan shown by `modn install <mod> --dry-run`.
use crate::catalog_install::{catalog_release, reference_of};
use anyhow::Result;
use modn_core::buckets::{BucketPackage, BucketStore, PackageSource};
use modn_core::planning::InstallPlan;
use modn_core::state::Store;
use modn_core::transactions::{CatalogDependency, DependencyState};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One release the selected mod needs, with its installed state.
pub(super) struct PlannedDependency {
    reference: String,
    version: String,
    sha256: String,
    source: PackageSource,
    state: DependencyState,
    /// A supplied archive whose digest matches this release.
    archive: Option<PathBuf>,
}

impl PlannedDependency {
    /// JSON record: the release fields plus the tagged `state` fields.
    fn as_json(&self) -> Result<serde_json::Value> {
        let mut record = serde_json::json!({
            "reference": self.reference,
            "version": self.version,
            "sha256": self.sha256,
            "source": self.source,
            "archive": self.archive,
        });
        if let (Some(record), serde_json::Value::Object(state)) =
            (record.as_object_mut(), serde_json::to_value(&self.state)?)
        {
            record.extend(state);
        }
        Ok(record)
    }

    pub(super) fn ready(&self) -> bool {
        matches!(self.state, DependencyState::Ready { .. })
    }

    pub(super) fn missing(&self) -> bool {
        matches!(self.state, DependencyState::Missing)
    }

    /// The state as one line, with what the user can do about it.
    pub(super) fn describe(&self) -> String {
        let state = match &self.state {
            DependencyState::Ready { package_ids } => format!("ready ({})", package_ids.join(", ")),
            DependencyState::Damaged { package_ids } => format!(
                "damaged ({}); restore or uninstall it, then install it again",
                package_ids.join(", ")
            ),
            DependencyState::OtherRelease { package_ids } => format!(
                "other release installed ({}); needs {}",
                package_ids.join(", "),
                self.version
            ),
            DependencyState::Missing => match &self.archive {
                Some(path) => format!("missing; archive supplied: {}", path.display()),
                None => format!("missing; {}", acquisition(&self.source)),
            },
            _ => "unknown state".to_owned(),
        };
        format!("{} {}: {state}", self.reference, self.version)
    }
}

/// The installed state of each dependency in `order`, the dependencies of
/// the selected release in install order.
///
/// Without manager state nothing is managed, so every dependency is missing.
pub(super) fn plan(
    order: &[BucketPackage],
    archives: &BTreeMap<String, PathBuf>,
    store: Option<&Store>,
    game: &Path,
) -> Result<Vec<PlannedDependency>> {
    let required: Vec<CatalogDependency> = order
        .iter()
        .map(|release| {
            CatalogDependency::new(reference_of(release), release.package.sha256.clone())
        })
        .collect();
    let states = match store {
        Some(store) => store.dependency_states(game, &required)?,
        None => vec![DependencyState::Missing; required.len()],
    };
    Ok(order
        .iter()
        .zip(states)
        .map(|(release, state)| {
            let reference = reference_of(release).to_string();
            PlannedDependency {
                archive: archives.get(&reference).cloned(),
                reference,
                version: release.package.version.clone(),
                sha256: release.package.sha256.clone(),
                source: release.package.source.clone(),
                state,
            }
        })
        .collect())
}

/// Installed packages that conflict with any release this command installs.
pub(super) fn conflicts(
    releases: &[&BucketPackage],
    catalog: &BucketStore,
    store: Option<&Store>,
    game: &Path,
) -> Result<Vec<String>> {
    let Some(store) = store else {
        return Ok(Vec::new());
    };
    let mut found = Vec::new();
    for release in releases {
        let reference = reference_of(release);
        let catalog_release = catalog_release(release, catalog)?;
        for package in
            store.conflicting_packages(game, &catalog_release, &release.package.sha256)?
        {
            found.push(format!("{reference} conflicts with installed {package}"));
        }
    }
    Ok(found)
}

/// How to obtain an archive, as a short instruction.
pub(super) fn acquisition(source: &PackageSource) -> String {
    match source {
        PackageSource::Manual { filename } => format!("download {filename:?} manually"),
        PackageSource::Https { url } => format!("Modn downloads it from {url}"),
        _ => "unsupported source".to_owned(),
    }
}

/// The selected archive's file operations, when the user supplied it.
pub(super) struct ArchivePreview<'a> {
    pub(super) plan: &'a InstallPlan,
    pub(super) unapproved_declared_executables: &'a [String],
}

/// Print a catalog dry run: dependencies first, then the selected archive.
///
/// Without `--file` the archive section names the archive to supply, and
/// `plan` and `unapproved_declared_executables` are `null` in JSON.
pub(super) fn report(
    reference: &str,
    selected: &BucketPackage,
    archive: Option<ArchivePreview<'_>>,
    dependencies: &[PlannedDependency],
    conflicts: &[String],
    json: bool,
) -> Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "package": reference,
                "version": selected.package.version,
                "source": selected.package.source,
                "plan": archive.as_ref().map(|preview| preview.plan),
                "unapproved_declared_executables":
                    archive.as_ref().map(|preview| preview.unapproved_declared_executables),
                "dependencies": dependencies
                    .iter()
                    .map(PlannedDependency::as_json)
                    .collect::<Result<Vec<_>>>()?,
                "dependencies_ready": dependencies.iter().all(PlannedDependency::ready),
                "conflicts": conflicts,
            }))?
        );
        return Ok(());
    }
    print(dependencies);
    for conflict in conflicts {
        println!("CONFLICT {conflict}");
    }
    let Some(preview) = archive else {
        let next = match &selected.package.source {
            PackageSource::Https { .. } => "when you install it",
            _ => "then add --file <archive>",
        };
        println!(
            "Dry run for {reference}: archive needed; {}, {next}",
            acquisition(&selected.package.source)
        );
        return Ok(());
    };
    println!("Dry run for {reference}: {}", preview.plan.status);
    for operation in &preview.plan.operations {
        println!("{} => {}", operation.source, operation.destination);
    }
    for issue in &preview.plan.blockers {
        println!("BLOCKER {}: {}", issue.code, issue.message);
    }
    for path in preview.unapproved_declared_executables {
        println!("DECLARED EXECUTABLE {path}: needs confirmation or --allow-executable");
    }
    Ok(())
}

/// Print the plan for a terminal, one dependency per line.
fn print(dependencies: &[PlannedDependency]) {
    if dependencies.is_empty() {
        return;
    }
    println!("Dependencies, in install order:");
    for dependency in dependencies {
        println!("  {}", dependency.describe());
    }
}
