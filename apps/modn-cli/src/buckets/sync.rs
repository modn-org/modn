//! Catalog sync helpers shared by `modn bucket sync` and `modn update`.

use anyhow::Result;
use modn_core::buckets::{
    BucketStore, CatalogKeyChange, CatalogKeyId, CatalogModuleReport, CatalogSigner,
    CatalogSyncTrace, GitCatalogSource, GitCatalogSync, ModuleIgnoreReason, ZipCatalogSource,
    ZipCatalogSync,
};
use std::fs::File;
use std::io::Read;

pub(crate) fn sync_catalog(
    store: &BucketStore,
    name: &str,
    source: Option<GitCatalogSource>,
    expected_fingerprint: Option<&CatalogKeyId>,
    json_output: bool,
) -> Result<GitCatalogSync> {
    let synced = store.sync_git_with_expected_fingerprint(name, source, expected_fingerprint)?;
    report_pin(name, &synced.revision.signer, synced.pinned, json_output);
    report_key_change(synced.key_change.as_ref(), json_output);
    report_ignored_key_statements(
        name,
        &synced.ignored_key_statements,
        synced.pinned,
        json_output,
    );
    report_trace(name, &synced.trace, json_output);
    Ok(synced)
}

pub(crate) fn sync_zip_catalog(
    store: &BucketStore,
    name: &str,
    source: Option<ZipCatalogSource>,
    expected_fingerprint: Option<&CatalogKeyId>,
    json_output: bool,
) -> Result<ZipCatalogSync> {
    let synced = store.sync_zip_with_expected_fingerprint(
        name,
        source,
        expected_fingerprint,
        open_zip_source,
    )?;
    report_pin(name, &synced.revision.signer, synced.pinned, json_output);
    report_key_change(synced.key_change.as_ref(), json_output);
    report_ignored_key_statements(
        name,
        &synced.ignored_key_statements,
        synced.pinned,
        json_output,
    );
    report_trace(name, &synced.trace, json_output);
    Ok(synced)
}

fn open_zip_source(source: &ZipCatalogSource) -> Result<Box<dyn Read>> {
    if source.url.starts_with("https://") {
        return crate::download::fetch_catalog(&source.url);
    }
    Ok(Box::new(File::open(&source.url)?))
}

fn report_pin(name: &str, signer: &CatalogSigner, pinned: bool, json_output: bool) {
    if !pinned {
        return;
    }
    let line = format!(
        "Pinned signer key {} for bucket {name}; compare this fingerprint with the catalog author.",
        signer.key_id
    );
    emit_report(&line, json_output);
    if let Some(key_id) = &signer.revocation_key_id {
        let line = format!(
            "Pinned revocation key {key_id} for bucket {name}; compare this fingerprint with the catalog author."
        );
        emit_report(&line, json_output);
    }
}

fn report_key_change(change: Option<&CatalogKeyChange>, json_output: bool) {
    if let Some(change) = change {
        emit_report(&change.line(), json_output);
    }
}

pub(crate) fn emit_report(line: &str, json_output: bool) {
    if json_output {
        eprintln!("{line}");
    } else {
        println!("{line}");
    }
}

fn report_trace(name: &str, trace: &CatalogSyncTrace, json_output: bool) {
    let mut lines = Vec::new();
    for failure in &trace.failed_sources {
        lines.push(format!("Source failed: {failure}"));
    }
    if trace.served_by != trace.primary {
        lines.push(format!(
            "Served bucket {name} from mirror {}.",
            trace.served_by.label()
        ));
    }
    if let (Some(from), Some(to)) = (&trace.moved_from, &trace.moved_to) {
        lines.push(format!(
            "catalog {name} moved from {} to {}.",
            from.label(),
            to.label()
        ));
    }
    for line in lines {
        if json_output {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    }
}

fn print_sync_modules(modules: &CatalogModuleReport) {
    for report in &modules.imported {
        println!(
            "Game module {} v{}: {}",
            report.game_id,
            report.version,
            serde_json::to_value(report.outcome)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_else(|| "unchanged".to_owned())
        );
    }
    for ignored in &modules.ignored {
        let reason = match ignored.reason {
            ModuleIgnoreReason::UntrustedSource => "the bucket is not a trusted module source",
            ModuleIgnoreReason::OfficialPriority => "the official main bucket provides this game",
        };
        println!("Ignored game module {}: {reason}.", ignored.game_id);
    }
}

pub(crate) fn print_git_sync(name: &str, synced: &GitCatalogSync) {
    println!("Synced bucket {name} at commit {}.", synced.revision.commit);
    print_sync_modules(&synced.modules);
}

fn report_ignored_key_statements(
    name: &str,
    statements: &[String],
    pinned: bool,
    json_output: bool,
) {
    if statements.is_empty() {
        return;
    }
    let kinds = statements.join(", ");
    let line = if pinned {
        format!(
            "Ignored published key change statements ({kinds}) for bucket {name}; the first sync pins the signer from catalog.pub and applies no statement."
        )
    } else {
        format!(
            "Ignored published key change statements ({kinds}) for bucket {name}; they do not name the pinned key."
        )
    };
    emit_report(&line, json_output);
}

pub(crate) fn print_zip_sync(name: &str, synced: &ZipCatalogSync) {
    println!(
        "Synced bucket {name} from {} at {}.",
        synced.revision.source.url, synced.revision.digest
    );
    print_sync_modules(&synced.modules);
}
