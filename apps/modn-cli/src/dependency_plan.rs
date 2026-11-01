//! Read-only dependency plan shown by `modn install <mod> --dry-run`.
use crate::catalog_install::reference_of;
use anyhow::Result;
use modn_core::buckets::{install_order, BucketPackage, BucketStore, PackageSource};
use modn_core::planning::InstallPlan;
use modn_core::state::Store;
use modn_core::transactions::{CatalogDependency, DependencyState};
use std::path::Path;

/// One release the selected mod needs, with its installed state.
pub(super) struct PlannedDependency {
    reference: String,
    version: String,
    sha256: String,
    source: PackageSource,
    state: DependencyState,
}

impl PlannedDependency {
    /// JSON record: the release fields plus the tagged `state` fields.
    fn as_json(&self) -> Result<serde_json::Value> {
        let mut record = serde_json::json!({
            "reference": self.reference,
            "version": self.version,
            "sha256": self.sha256,
            "source": self.source,
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
}

/// Every dependency of `selected`, transitively, in install order.
///
/// Without manager state nothing is managed, so every dependency is missing.
pub(super) fn plan(
    selected: &BucketPackage,
    catalog: &BucketStore,
    store: Option<&Store>,
    game: &Path,
) -> Result<Vec<PlannedDependency>> {
    let mut order = install_order(selected, &catalog.packages(Some(&selected.package.game))?)?;
    order.pop(); // The selected release is last; it is not its own dependency.
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
        .into_iter()
        .zip(states)
        .map(|(release, state)| PlannedDependency {
            reference: reference_of(&release).to_string(),
            version: release.package.version,
            sha256: release.package.sha256,
            source: release.package.source,
            state,
        })
        .collect())
}

/// How to obtain an archive, as a short instruction.
pub(super) fn acquisition(source: &PackageSource) -> String {
    match source {
        PackageSource::Manual { filename } => format!("download {filename:?} manually"),
        PackageSource::Https { url } => format!("download from {url}"),
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
            }))?
        );
        return Ok(());
    }
    print(dependencies);
    let Some(preview) = archive else {
        println!(
            "Dry run for {reference}: archive needed; {}, then add --file <archive>",
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
        let state = match &dependency.state {
            DependencyState::Ready { package_ids } => format!("ready ({})", package_ids.join(", ")),
            DependencyState::Damaged { package_ids } => format!(
                "damaged ({}); restore or uninstall it, then install it again",
                package_ids.join(", ")
            ),
            DependencyState::OtherRelease { package_ids } => format!(
                "other release installed ({}); needs {}",
                package_ids.join(", "),
                dependency.version
            ),
            DependencyState::Missing => format!("missing; {}", acquisition(&dependency.source)),
            _ => "unknown state".to_owned(),
        };
        println!("  {} {}: {state}", dependency.reference, dependency.version);
    }
}
