//! Text and JSON rendering of the catalog `install --dry-run` report.
//!
//! Every fact comes from the `CatalogPlan` and the preview the CLI just ran.
//! Field order, labels and JSON keys are the shipped contract.

use crate::dependency_plan::{
    self, acquisition, fetchable_source, planned_source, source_labels, PlannedDependency,
};
use crate::download;
use crate::install_consent::ApprovalCheck;
use anyhow::{Context, Result};
use modn_core::acquisition::SourceAttempt;
use modn_core::buckets::PackageSource;
use modn_core::catalog_pipeline::{CatalogPlan, ReleasePreview};

pub(super) struct ArchivePreview<'a> {
    pub(super) preview: &'a ReleasePreview,
    pub(super) unapproved_declared_executables: &'a [String],
}

pub(super) struct ReportInput<'a> {
    pub(super) reference: &'a str,
    pub(super) version: &'a str,
    pub(super) sources: &'a [PackageSource],
    pub(super) primary: &'a PackageSource,
}

/// Print the dry-run report of `plan`.
pub(super) fn print(
    plan: &CatalogPlan,
    archive: Option<ArchivePreview<'_>>,
    approval: Option<&ApprovalCheck>,
    downloads: &[SourceAttempt],
    json: bool,
) -> Result<()> {
    let selected = plan
        .order
        .last()
        .context("catalog plan ends with the selected release")?;
    let reference = plan.selected.to_string();
    let dependencies = dependency_plan::dependencies(plan);
    let conflicts = conflict_lines(plan);
    let report = ReportInput {
        reference: &reference,
        version: &selected.package.package.version,
        sources: &selected.package.package.sources,
        primary: selected.package.package.primary_source(),
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report_json(
                &report,
                archive,
                &dependencies,
                &conflicts,
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
            &dependencies,
            &conflicts,
            approval,
            downloads,
        )
    );
    Ok(())
}

/// The report's conflict lines, in plan order.
pub(super) fn conflict_lines(plan: &CatalogPlan) -> Vec<String> {
    plan.conflicts
        .iter()
        .flat_map(|conflict| {
            conflict.installed.iter().map(move |package| {
                format!("{} conflicts with installed {package}", conflict.reference)
            })
        })
        .collect()
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
        "source": planned_source(report.sources, report.primary).clone(),
        "sources": source_labels(report.sources),
        "plan": archive.as_ref().map(|archive| &archive.preview.plan),
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
    print_dependencies(dependencies, &mut out);
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
        report.reference, preview.preview.plan.status
    );
    for operation in &preview.preview.plan.operations {
        let _ = writeln!(out, "{} => {}", operation.source, operation.destination);
    }
    for issue in &preview.preview.plan.blockers {
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

fn print_dependencies(dependencies: &[PlannedDependency], out: &mut String) {
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
