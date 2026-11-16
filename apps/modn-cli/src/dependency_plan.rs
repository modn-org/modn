use crate::catalog_install::reference_of;
use crate::download;
use crate::install_consent::ApprovalCheck;
use anyhow::Result;
use modn_core::acquisition::SourceAttempt;
use modn_core::buckets::{BucketPackage, BucketSnapshot, PackageSource};
use modn_core::planning::InstallPlan;
use modn_core::state::Store;
use modn_core::transactions::{CatalogDependency, DependencyState};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(super) struct PlannedDependency {
    reference: String,
    version: String,
    blake3: String,
    source: PackageSource,
    state: DependencyState,
    archive: Option<PathBuf>,
}

impl PlannedDependency {
    fn as_json(&self) -> Result<serde_json::Value> {
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
            CatalogDependency::new(
                reference_of(release),
                download::digest(release, digests)
                    .unwrap_or_default()
                    .to_owned(),
            )
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
                blake3: download::digest(release, digests)
                    .unwrap_or_default()
                    .to_owned(),
                source: release.package.primary_source().clone(),
                state,
            }
        })
        .collect())
}

pub(super) fn conflicts(
    releases: &[&BucketPackage],
    catalog: &BucketSnapshot,
    digests: &BTreeMap<String, String>,
    store: Option<&Store>,
    game: &Path,
) -> Result<Vec<String>> {
    let Some(store) = store else {
        return Ok(Vec::new());
    };
    let mut found = Vec::new();
    for release in releases {
        let reference = reference_of(release);
        let catalog_release = catalog.release(release)?;
        for package in store.conflicting_packages(
            game,
            Some(&catalog_release),
            download::digest(release, digests).unwrap_or_default(),
        )? {
            found.push(format!("{reference} conflicts with installed {package}"));
        }
    }
    Ok(found)
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

/// One ordered source, as the plan names it.
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
        } => format!("https://github.com/{repository}/releases/download/{tag}/{asset}"),
        PackageSource::Git { repository, commit } => format!("git {repository} commit {commit}"),
        _ => "unsupported source".to_owned(),
    }
}

/// Every ordered source, in the order the acquisition loop tries them.
pub(super) fn source_labels(sources: &[PackageSource]) -> Vec<String> {
    sources.iter().map(source_label).collect()
}

/// The first source that Modn can fetch without the user supplying an archive.
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

/// The source that the acquisition loop will try first, or the primary source.
fn planned_source<'a>(
    sources: &'a [PackageSource],
    primary: &'a PackageSource,
) -> &'a PackageSource {
    fetchable_source(sources).unwrap_or(primary)
}

pub(super) struct ArchivePreview<'a> {
    pub(super) plan: &'a InstallPlan,
    pub(super) unapproved_declared_executables: &'a [String],
}

/// The release facts that a dry-run report prints.
pub(super) struct ReportInput<'a> {
    pub(super) reference: &'a str,
    pub(super) version: &'a str,
    pub(super) sources: &'a [PackageSource],
    pub(super) primary: &'a PackageSource,
}

impl<'a> ReportInput<'a> {
    fn of(reference: &'a str, selected: &'a BucketPackage) -> Self {
        Self {
            reference,
            version: &selected.package.version,
            sources: &selected.package.sources,
            primary: selected.package.primary_source(),
        }
    }
}

pub(super) fn report(
    reference: &str,
    selected: &BucketPackage,
    archive: Option<ArchivePreview<'_>>,
    dependencies: &[PlannedDependency],
    conflicts: &[String],
    approval: Option<&ApprovalCheck>,
    downloads: &[SourceAttempt],
    json: bool,
) -> Result<()> {
    let report = ReportInput::of(reference, selected);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report_json(
                &report,
                archive,
                dependencies,
                conflicts,
                approval,
                downloads,
            )?)?
        );
        return Ok(());
    }
    print!(
        "{}",
        report_text(
            &report,
            archive,
            dependencies,
            conflicts,
            approval,
            downloads,
        )
    );
    Ok(())
}

fn report_json(
    report: &ReportInput<'_>,
    archive: Option<ArchivePreview<'_>>,
    dependencies: &[PlannedDependency],
    conflicts: &[String],
    approval: Option<&ApprovalCheck>,
    downloads: &[SourceAttempt],
) -> Result<serde_json::Value> {
    Ok(serde_json::json!({
        "package": report.reference,
        "version": report.version,
        "source": report.primary.clone(),
        "sources": source_labels(report.sources),
        "plan": archive.as_ref().map(|preview| preview.plan),
        "archive_needed": archive
            .is_none()
            .then(|| acquisition(planned_source(report.sources, report.primary))),
        "unapproved_declared_executables":
            archive.as_ref().map(|preview| preview.unapproved_declared_executables),
        "downloads": download::downloads_json(downloads),
        "dependencies": dependencies
            .iter()
            .map(PlannedDependency::as_json)
            .collect::<Result<Vec<_>>>()?,
        "dependencies_ready": dependencies.iter().all(PlannedDependency::ready),
        "conflicts": conflicts,
        "approval": approval.map(ApprovalCheck::as_json),
    }))
}

fn report_text(
    report: &ReportInput<'_>,
    archive: Option<ArchivePreview<'_>>,
    dependencies: &[PlannedDependency],
    conflicts: &[String],
    approval: Option<&ApprovalCheck>,
    downloads: &[SourceAttempt],
) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    print(dependencies, &mut out);
    for conflict in conflicts {
        let _ = writeln!(out, "CONFLICT {conflict}");
    }
    for attempt in downloads {
        let _ = writeln!(out, "DOWNLOAD {}: {}", attempt.source, attempt.describe());
    }
    let Some(preview) = archive else {
        let planned = planned_source(report.sources, report.primary);
        let next = match fetchable_source(report.sources) {
            Some(_) => "when you install it",
            None => "then add --file <archive>",
        };
        let _ = writeln!(out, "Sources, in order:");
        for label in source_labels(report.sources) {
            let _ = writeln!(out, "  {label}");
        }
        let _ = writeln!(
            out,
            "Dry run for {}: archive needed; {}, {next}",
            report.reference,
            acquisition(planned)
        );
        return out;
    };
    let _ = writeln!(
        out,
        "Dry run for {}: {}",
        report.reference, preview.plan.status
    );
    for operation in &preview.plan.operations {
        let _ = writeln!(out, "{} => {}", operation.source, operation.destination);
    }
    for issue in &preview.plan.blockers {
        let _ = writeln!(out, "BLOCKER {}: {}", issue.code, issue.message);
    }
    for path in preview.unapproved_declared_executables {
        let _ = writeln!(
            out,
            "DECLARED EXECUTABLE {path}: needs confirmation or --allow-executable"
        );
    }
    if let Some(approval) = approval {
        if approval.uncovered.is_empty() {
            let _ = writeln!(
                out,
                "Approval: required capabilities are covered by the stored approval."
            );
        } else {
            if approval.uncovered.native_injector {
                let _ = writeln!(
                    out,
                    "APPROVAL REQUIRED native injector: pass --allow-native-injector"
                );
            }
            for path in &approval.uncovered.executables {
                let _ = writeln!(
                    out,
                    "APPROVAL REQUIRED {path}: pass --allow-executable or confirm interactively"
                );
            }
        }
    }
    out
}

fn print(dependencies: &[PlannedDependency], out: &mut String) {
    use std::fmt::Write as _;
    if dependencies.is_empty() {
        return;
    }
    let _ = writeln!(out, "Dependencies, in install order:");
    for dependency in dependencies {
        let _ = writeln!(out, "  {}", dependency.describe());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use modn_core::acquisition::SourceFailure;

    fn sources() -> Vec<PackageSource> {
        serde_json::from_value(serde_json::json!([
            {"kind": "manual", "filename": "Example 1.0.zip"},
            {"kind": "https", "url": "https://example.invalid/example.zip"}
        ]))
        .expect("sources")
    }

    fn attempt() -> SourceAttempt {
        SourceAttempt::failed(
            "https://example.invalid/example.zip",
            2,
            SourceFailure::Network("connection reset".to_owned()),
        )
    }

    #[test]
    fn a_report_lists_every_download_attempt() {
        let sources = sources();
        let report = ReportInput {
            reference: "community/test-game/example",
            version: "1.0",
            sources: &sources,
            primary: &sources[0],
        };
        let value = report_json(&report, None, &[], &[], None, &[attempt()]).expect("report");
        assert_eq!(value["downloads"][0]["attempt"], 2);
        assert_eq!(value["downloads"][0]["outcome"], "failed");
        assert_eq!(
            value["downloads"][0]["source"],
            "https://example.invalid/example.zip"
        );
        assert!(value["downloads"][0]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("connection reset")));

        let text = report_text(&report, None, &[], &[], None, &[attempt()]);
        assert!(
            text.contains("DOWNLOAD https://example.invalid/example.zip: attempt 2: network: connection reset"),
            "{text}"
        );
    }

    #[test]
    fn a_report_names_the_first_fetchable_source() {
        let sources = sources();
        let report = ReportInput {
            reference: "community/test-game/example",
            version: "1.0",
            sources: &sources,
            primary: &sources[0],
        };
        let value = report_json(&report, None, &[], &[], None, &[]).expect("report");
        let needed = value["archive_needed"].as_str().expect("archive needed");
        assert!(
            needed.contains("https://example.invalid/example.zip"),
            "{needed}"
        );
        let text = report_text(&report, None, &[], &[], None, &[]);
        assert!(text.contains("when you install it"), "{text}");
        assert!(!text.contains("then add --file <archive>"), "{text}");
    }
}
