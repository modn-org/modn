//! CLI views over the dependency rows and artifact sources of a catalog plan.
//!
//! The plan lives in `modn_core::catalog_pipeline`. This module keeps the
//! dependency rows the stack step and the dry-run report already render, and
//! the acquisition text that errors and prompts already print. It never reads
//! the store or the archive cache on the install path.

use crate::catalog_install::reference_of;
use crate::download;
use anyhow::Result;
use modn_core::acquisition::GithubReleaseAsset;
use modn_core::buckets::{BucketPackage, PackageSource};
use modn_core::catalog_pipeline::{CatalogPlan, PlannedRelease};
use modn_core::state::Store;
use modn_core::transactions::{CatalogDependency, DependencyState};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(super) struct PlannedDependency {
    reference: String,
    version: String,
    blake3: Option<String>,
    source: PackageSource,
    state: DependencyState,
    archive: Option<PathBuf>,
}

impl PlannedDependency {
    fn from_planned(release: &PlannedRelease) -> Self {
        let package = &release.package.package;
        Self {
            reference: release.package.reference().to_string(),
            version: package.version.clone(),
            blake3: release.digest.as_str().map(str::to_owned),
            source: planned_source(&package.sources, package.primary_source()).clone(),
            state: release.state.clone().unwrap_or(DependencyState::Missing),
            archive: release.served.as_ref().map(|served| served.path.clone()),
        }
    }

    pub(super) fn as_json(&self) -> Result<serde_json::Value> {
        let mut record = serde_json::json!({
            "reference": self.reference,
            "version": self.version,
            "blake3": self.blake3,
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

/// The dependency rows of one plan, in install order. The selected release is
/// not a dependency row.
pub(super) fn dependencies(plan: &CatalogPlan) -> Vec<PlannedDependency> {
    plan.order
        .iter()
        .take(plan.order.len().saturating_sub(1))
        .map(PlannedDependency::from_planned)
        .collect()
}

pub(super) fn plan(
    order: &[BucketPackage],
    archives: &BTreeMap<String, PathBuf>,
    digests: &BTreeMap<String, String>,
    store: Option<&Store>,
    game: &Path,
) -> Result<Vec<PlannedDependency>> {
    let required: Vec<CatalogDependency> = order
        .iter()
        .map(|release| {
            Ok(CatalogDependency::with_digest(
                reference_of(release),
                download::digest(release, digests)?,
            ))
        })
        .collect::<Result<_>>()?;
    let states = match store {
        Some(store) => store.dependency_states(game, &required)?,
        None => vec![DependencyState::Missing; required.len()],
    };
    order
        .iter()
        .zip(states)
        .map(|(release, state)| {
            let digest = download::digest(release, digests)?;
            let reference = reference_of(release).to_string();
            Ok(PlannedDependency {
                archive: archives.get(&reference).cloned(),
                reference,
                version: release.package.version.clone(),
                blake3: digest.as_str().map(str::to_owned),
                source: planned_source(&release.package.sources, release.package.primary_source())
                    .clone(),
                state,
            })
        })
        .collect()
}

pub(super) fn acquisition(source: &PackageSource) -> String {
    match source {
        PackageSource::Manual { filename } => format!("download {filename:?} manually"),
        PackageSource::Nexus {
            game_domain,
            mod_id,
            file_id,
            file_name,
        } => format!(
            "Modn downloads from Nexus Mods: {game_domain} mod {mod_id} file {file_id} ({file_name})"
        ),
        PackageSource::Https { url } => format!("Modn downloads it from {url}"),
        PackageSource::GithubRelease {
            repository,
            tag,
            asset,
        } => format!("Modn resolves GitHub release {repository} {tag} asset {asset}"),
        PackageSource::Git { repository, commit } => {
            format!("Modn needs to download pinned commit {commit} from {repository}")
        }
        _ => "unsupported source".to_owned(),
    }
}

pub(super) fn source_label(source: &PackageSource) -> String {
    match source {
        PackageSource::Manual { filename } => format!("manual: {filename}"),
        PackageSource::Nexus {
            game_domain,
            mod_id,
            file_id,
            file_name,
        } => format!("Nexus Mods: {game_domain} mod {mod_id} file {file_id} ({file_name})"),
        PackageSource::Https { url } => url.clone(),
        PackageSource::GithubRelease {
            repository,
            tag,
            asset,
        } => GithubReleaseAsset::new(repository.clone(), tag.clone(), asset.clone()).download_url(),
        PackageSource::Git { repository, commit } => format!("git {repository} commit {commit}"),
        _ => "unsupported source".to_owned(),
    }
}

pub(super) fn source_labels(sources: &[PackageSource]) -> Vec<String> {
    sources.iter().map(source_label).collect()
}

pub(super) fn fetchable_source(sources: &[PackageSource]) -> Option<&PackageSource> {
    sources.iter().find(|source| {
        matches!(
            source,
            PackageSource::Https { .. }
                | PackageSource::GithubRelease { .. }
                | PackageSource::Git { .. }
                | PackageSource::Nexus { .. }
        )
    })
}

pub(super) fn planned_source<'a>(
    sources: &'a [PackageSource],
    primary: &'a PackageSource,
) -> &'a PackageSource {
    fetchable_source(sources).unwrap_or(primary)
}

pub(super) fn download_line(
    reference: &str,
    version: &str,
    sources: &[PackageSource],
    primary: &PackageSource,
) -> String {
    format!(
        "{reference} {version}; {}",
        acquisition(planned_source(sources, primary))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dependency_reports_a_known_digest_and_null_for_an_unknown_one() {
        let digest = "a".repeat(64);
        for (blake3, expected) in [
            (Some(digest.clone()), serde_json::json!(digest)),
            (None, serde_json::Value::Null),
        ] {
            let dependency = PlannedDependency {
                reference: "community/test-game/example".to_owned(),
                version: "1.0".to_owned(),
                blake3,
                source: PackageSource::Manual {
                    filename: "example.zip".to_owned(),
                },
                state: DependencyState::Missing,
                archive: None,
            };
            let value = dependency.as_json().expect("record");
            assert_eq!(value["blake3"], expected);
        }
    }
}
