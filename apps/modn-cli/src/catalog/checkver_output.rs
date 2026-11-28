//! Human and JSON reports of `modn catalog checkver`.

use super::checkver::{Marker, Report, Status, Summary};
use serde_json::{json, Value};

pub(super) const REMINDER: &str = "Run 'modn catalog sign' and 'modn catalog verify' before you publish; checkver never changes catalog.sig.";
pub(super) const NEXUS_UPDATE_UNSUPPORTED: &str =
    "nexus checkver update is not supported: update the manifest by hand";

pub(super) fn print_json(reports: &[Report], summary: &Summary, update: bool) {
    let packages: Vec<Value> = reports
        .iter()
        .map(|report| {
            let mut value = json!({
                "game": report.game,
                "id": report.id,
                "status": report.status.as_str(),
                "kind": report.kind,
                "current_version": report.current_version,
                "latest_version": report.latest_version,
                "error": report.error,
            });
            if update {
                value["updated"] = json!(report.applied.is_some());
                value["kept_previous"] = json!(report
                    .applied
                    .as_ref()
                    .and_then(|applied| applied.kept_previous.clone()));
            }
            value
        })
        .collect();
    let mut value = json!({
        "packages": packages,
        "summary": {
            "up_to_date": summary.up_to_date,
            "update_available": summary.update_available,
            "skipped": summary.skipped,
            "error": summary.error,
        },
    });
    if update {
        value["summary"]["updated"] = json!(summary.updated);
        if summary.updated > 0 {
            value["reminder"] = json!(REMINDER);
        }
    }
    println!("{value}");
}

pub(super) fn print_human(reports: &[Report], summary: &Summary, update: bool) {
    for report in reports {
        let name = format!("{}/{}", report.game, report.id);
        match report.status {
            Status::Error => println!(
                "{name}: error: {}",
                report.error.as_deref().unwrap_or("unknown error")
            ),
            Status::Skipped => println!(
                "{name}: skipped {}",
                report.current_version.as_deref().unwrap_or("?")
            ),
            status => println!(
                "{name}: {} {} -> {}",
                status.as_str(),
                report.current_version.as_deref().unwrap_or("?"),
                report.latest_version.as_deref().unwrap_or("unknown")
            ),
        }
    }
    if update {
        print_updates(reports);
    }
    println!(
        "Summary: {} up_to_date, {} update_available, {} skipped, {} error",
        summary.up_to_date, summary.update_available, summary.skipped, summary.error
    );
    if update && summary.updated > 0 {
        println!("{REMINDER}");
    }
}

fn print_updates(reports: &[Report]) {
    for report in reports {
        let Some(applied) = &report.applied else {
            continue;
        };
        let name = format!("{}/{}", report.game, report.id);
        let marker = match &applied.marker {
            Marker::Blake3(hex) => format!("blake3 {}...", first(hex)),
            Marker::Commit(commit) => format!("commit {}...", first(commit)),
        };
        println!(
            "updated {name}: {} -> {} ({marker})",
            applied.old_version, applied.new_version
        );
        if let Some(kept) = &applied.kept_previous {
            println!("kept previous manifest: {kept}");
        }
        if applied.removed_upstream_sha256 {
            println!("warning: {name}: removed upstream_sha256, which described the old artifact");
        }
        if applied.other_sources > 0 {
            println!(
                "warning: {name}: {} other source(s) were not updated and no longer match the new pin",
                applied.other_sources
            );
        }
    }
}

fn first(value: &str) -> String {
    value.chars().take(12).collect()
}
