use crate::commands::BucketCommand;
use crate::state_access::validate_write_targets;
use anyhow::{anyhow, bail, Context, Result};
use modn_core::buckets::{
    local_bucket_module_refusal, BucketStore, CatalogKeyId, GitCatalogSource, KnownBucket,
    KnownSourceKind, ModuleTrustAck, ModuleTrustState, ZipCatalogSource,
};
use modn_core::manager_roots::ManagerRoots;
use serde_json::Value;

pub(crate) use self::report::{catalog as catalog_report, CatalogReport};
pub(super) use self::sync::{
    emit_report, print_git_sync, print_zip_sync, sync_catalog, sync_zip_catalog,
};

mod known;
mod report;
mod sync;

fn host_label(host: &Value) -> String {
    match host.get("kind").and_then(Value::as_str) {
        Some("zip") => format!(
            "zip {}",
            host.get("url").and_then(Value::as_str).unwrap_or("")
        ),
        Some("git") => format!(
            "git {}@{}",
            host.get("remote").and_then(Value::as_str).unwrap_or(""),
            host.get("revision").and_then(Value::as_str).unwrap_or("")
        ),
        _ => String::new(),
    }
}
enum CatalogKind {
    Git(Option<GitCatalogSource>),
    Zip(Option<ZipCatalogSource>),
}
struct CatalogRequest {
    kind: CatalogKind,
    expected_fingerprint: Option<CatalogKeyId>,
}
fn classify_source(
    store: &BucketStore,
    name: &str,
    source: Option<&str>,
    revision: Option<&str>,
) -> Result<CatalogRequest> {
    match (source, revision) {
        (Some(text), _) if ZipCatalogSource::selects(text) => {
            if revision.is_some() {
                bail!(
                    "`--revision` cannot be used with a ZIP catalog source; a ZIP source has no Git revision"
                );
            }
            Ok(CatalogRequest {
                kind: CatalogKind::Zip(Some(ZipCatalogSource::new(text.to_owned()))),
                expected_fingerprint: None,
            })
        }
        (Some(text), Some(revision)) => Ok(CatalogRequest {
            kind: CatalogKind::Git(Some(GitCatalogSource::new(
                text.to_owned(),
                revision.to_owned(),
            ))),
            expected_fingerprint: None,
        }),
        (Some(_), None) => {
            bail!("a Git catalog source needs `--revision`; a ZIP catalog source ends in .zip")
        }
        (None, Some(_)) => bail!(
            "`--revision` needs `--source`; a known bucket takes its revision from the main bucket list"
        ),
        (None, None) => {
            if store.zip_revision(name)?.is_some() {
                return Ok(CatalogRequest {
                    kind: CatalogKind::Zip(None),
                    expected_fingerprint: None,
                });
            }
            if store.git_revision(name)?.is_some() {
                return Ok(CatalogRequest {
                    kind: CatalogKind::Git(None),
                    expected_fingerprint: None,
                });
            }
            let known = store.known_buckets()?;
            let Some(entry) = known.get(name) else {
                return Err(anyhow!(
                    "provide a source for registration; changing a registered source is refused"
                ))
                .context("modn bucket known lists the known buckets");
            };
            known_request(entry)
        }
    }
}

fn known_request(entry: &KnownBucket) -> Result<CatalogRequest> {
    let kind = match entry.kind {
        KnownSourceKind::Git => CatalogKind::Git(Some(GitCatalogSource::new(
            entry.source.clone(),
            entry.revision.clone().unwrap_or_else(|| "HEAD".to_owned()),
        ))),
        KnownSourceKind::Zip => CatalogKind::Zip(Some(ZipCatalogSource::new(entry.source.clone()))),
        _ => bail!(
            "known bucket {} uses a source kind that this Modn build does not support",
            entry.name
        ),
    };
    Ok(CatalogRequest {
        kind,
        expected_fingerprint: Some(entry.fingerprint),
    })
}
fn module_signer(store: &BucketStore, bucket: &str) -> Result<String> {
    if bucket == "main" {
        bail!("the official main bucket is always a module source and needs no trust record");
    }
    match store.module_trust(bucket)? {
        ModuleTrustState::Local => {
            bail!("{}", local_bucket_module_refusal(bucket))
        }
        ModuleTrustState::Main => bail!("the official main bucket is always a module source"),
        ModuleTrustState::Trusted | ModuleTrustState::Untrusted => Ok(store
            .catalog_signer(bucket)?
            .context("the bucket has no pinned signer")?
            .key_id),
        _ => bail!("unsupported module trust state for bucket {bucket}"),
    }
}

fn trust_modules(
    store: &BucketStore,
    bucket: &str,
    yes: bool,
    fingerprint: Option<&str>,
    json_output: bool,
) -> Result<()> {
    let signer = module_signer(store, bucket)?;
    if let Some(expected) = fingerprint {
        if expected != signer {
            bail!(
                "signer fingerprint mismatch for bucket {bucket}: expected {expected}, pinned signer is {signer}"
            );
        }
    }
    if !yes {
        bail!(
            "bucket {bucket} is signed by {signer}; modules from this bucket run as game policy, so Modn executes publisher code. Re-run with --yes to confirm, optionally with --fingerprint {signer}."
        );
    }
    let outcome = store.trust_modules(
        bucket,
        ModuleTrustAck::Acknowledged {
            expected_fingerprint: fingerprint,
        },
    )?;
    if json_output {
        let imported: Vec<Value> = outcome
            .imported
            .iter()
            .map(|report| {
                serde_json::json!({
                    "game_id": report.game_id,
                    "version": report.version,
                    "outcome": report.outcome,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "bucket": outcome.bucket,
                "signer_key_id": outcome.signer_key_id,
                "imported": imported,
            }))?
        );
    } else {
        println!(
            "Trusted signer {} for bucket {}; modules from this bucket are executable policy.",
            outcome.signer_key_id, outcome.bucket
        );
    }
    Ok(())
}

fn untrust_modules(store: &BucketStore, bucket: &str, json_output: bool) -> Result<()> {
    store.untrust_modules(bucket)?;
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "bucket": bucket,
                "module_trust": "removed",
            }))?
        );
    } else {
        println!(
            "Removed module trust for bucket {bucket}; its modules are ignored at sync and refused at use."
        );
    }
    Ok(())
}

pub(super) fn run(
    command: BucketCommand,
    roots: &ManagerRoots,
    json_output: bool,
    game: Option<&str>,
) -> Result<()> {
    let store = BucketStore::new(roots);
    if matches!(
        &command,
        BucketCommand::Add { .. }
            | BucketCommand::Remove { .. }
            | BucketCommand::Sync { .. }
            | BucketCommand::TrustModules { .. }
            | BucketCommand::UntrustModules { .. }
    ) {
        validate_write_targets(roots, game)?;
    }
    let value: Value = match command {
        BucketCommand::Sync {
            name,
            source,
            revision,
        } => {
            let request = classify_source(&store, &name, source.as_deref(), revision.as_deref())?;
            let known_sync = request.expected_fingerprint.is_some();
            let value = match request.kind {
                CatalogKind::Git(source) => {
                    let synced = sync_catalog(
                        &store,
                        roots,
                        &name,
                        source,
                        request.expected_fingerprint.as_ref(),
                        json_output,
                    )?;
                    if !json_output {
                        print_git_sync(&name, &synced);
                    }
                    let mut value = serde_json::to_value(&synced)?;
                    if let Some(sequence) = catalog_report(roots, &name, &synced.warnings).sequence
                    {
                        value["sequence"] = serde_json::json!(sequence);
                    }
                    value
                }
                CatalogKind::Zip(source) => {
                    let synced = sync_zip_catalog(
                        &store,
                        roots,
                        &name,
                        source,
                        request.expected_fingerprint.as_ref(),
                        json_output,
                    )?;
                    if !json_output {
                        print_zip_sync(&name, &synced);
                    }
                    let mut value = serde_json::to_value(&synced)?;
                    if let Some(sequence) = catalog_report(roots, &name, &synced.warnings).sequence
                    {
                        value["sequence"] = serde_json::json!(sequence);
                    }
                    value
                }
            };
            if known_sync {
                emit_report(
                    &format!(
                        "Synced known bucket {name}; module trust was not granted. Run `modn bucket trust-modules {name}` to trust its modules."
                    ),
                    json_output,
                );
            }
            value
        }
        BucketCommand::Known => {
            return known::run(roots, json_output);
        }
        BucketCommand::TrustModules {
            bucket,
            yes,
            fingerprint,
        } => {
            return trust_modules(&store, &bucket, yes, fingerprint.as_deref(), json_output);
        }
        BucketCommand::UntrustModules { bucket } => {
            return untrust_modules(&store, &bucket, json_output);
        }
        BucketCommand::Update { name } => {
            return crate::bucket_update::run(name.as_deref(), roots, json_output);
        }
        BucketCommand::Add { name, path } => serde_json::to_value(store.add(&name, &path)?)?,
        BucketCommand::Remove { name } => {
            store.remove(&name)?;
            serde_json::json!({"removed": name})
        }
        BucketCommand::List => serde_json::to_value(store.list()?)?,
        BucketCommand::Show { package } => serde_json::to_value(store.show(&package)?)?,
    };
    if json_output {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        match command_description(&value) {
            Some(lines) => {
                for line in lines {
                    println!("{line}");
                }
            }
            None => println!("{}", serde_json::to_string_pretty(&value)?),
        }
    }
    Ok(())
}

fn command_description(value: &Value) -> Option<Vec<String>> {
    let array = value.as_array()?;
    Some(
        array
            .iter()
            .map(|entry| {
                let prefix = entry
                    .get("bucket")
                    .and_then(Value::as_str)
                    .unwrap_or_else(|| entry.get("name").and_then(Value::as_str).unwrap_or(""));
                let game = entry.get("game").or_else(|| entry.get("game_id"));
                let id = entry.get("id");
                match (game, id, entry.get("version")) {
                    (Some(game), Some(id), Some(version)) => format!(
                        "{prefix}/{}/{}, version {}",
                        game.as_str().unwrap_or(""),
                        id.as_str().unwrap_or(""),
                        version.as_str().unwrap_or("")
                    ),
                    (Some(game), None, Some(version)) => format!(
                        "{prefix}/{}, version {}",
                        game.as_str().unwrap_or(""),
                        version.as_str().unwrap_or("")
                    ),
                    _ => {
                        let mut line = format!(
                            "{prefix}: {}",
                            entry.get("title").and_then(Value::as_str).unwrap_or("")
                        );
                        if let Some(key_id) = entry.get("signer_key_id").and_then(Value::as_str) {
                            line.push_str(&format!(" [signer {key_id}]"));
                        }
                        if let Some(primary) = entry.get("primary") {
                            let mirrors = entry
                                .get("mirrors")
                                .and_then(Value::as_array)
                                .map_or(0, Vec::len);
                            line.push_str(&format!(
                                " [primary {}, {mirrors} mirrors]",
                                host_label(primary)
                            ));
                        }
                        line
                    }
                }
            })
            .collect(),
    )
}

pub(super) fn search(
    query: Option<&str>,
    roots: &ManagerRoots,
    json_output: bool,
    game: Option<&str>,
) -> Result<()> {
    let catalog = BucketStore::new(roots);
    let needle = query.unwrap_or("").to_lowercase();
    let matches: Vec<_> = catalog
        .packages(game)?
        .into_iter()
        .filter(|entry| {
            entry.package.id.to_lowercase().contains(&needle)
                || entry.package.name.to_lowercase().contains(&needle)
        })
        .collect();
    let value = serde_json::to_value(matches)?;
    if json_output {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        for line in command_description(&value).unwrap_or_default() {
            println!("{line}");
        }
    }
    Ok(())
}
