use crate::commands::BucketCommand;
use crate::state_access::{validate_state_path, ManagerState};
use anyhow::{bail, Context, Result};
use modn_core::buckets::{
    local_bucket_module_refusal, BucketStore, GitCatalogSource, GitCatalogSync, ModuleIgnoreReason,
    ModuleTrustAck, ModuleTrustState,
};
use modn_core::state::Store;
use serde_json::Value;
use std::path::Path;

/// Refuse a registry write when the state directory sits inside a game installation.
pub(super) fn validate_registry_write(
    state_path: &Path,
    root: &Path,
    store: &BucketStore,
    game: Option<&str>,
) -> Result<()> {
    let state = ManagerState::load_deferred(state_path)?;
    let registered = state
        .existing()
        .map(Store::list)
        .transpose()?
        .unwrap_or_default();
    let roots = registered
        .into_iter()
        .map(|game| game.path)
        .collect::<Vec<_>>();
    let mut games = store.official_games()?;
    if let Some(game) = game {
        games.push(game.to_owned());
    }
    games.sort();
    games.dedup();
    let registry_target = root.join("buckets/registrations/.bucket-entry.json");
    if games.is_empty() {
        validate_state_path(&registry_target, None, None, &roots)?;
    }
    for game in games {
        validate_state_path(&registry_target, Some(&game), None, &roots)?;
    }
    Ok(())
}

/// Fetch and verify one registered Git catalog, reporting a newly pinned signer.
pub(super) fn sync_catalog(
    store: &BucketStore,
    name: &str,
    source: Option<GitCatalogSource>,
    json_output: bool,
) -> Result<GitCatalogSync> {
    let synced = store.sync_git_with_signer(name, source)?;
    if synced.pinned {
        let line = format!(
            "Pinned signer key {} for bucket {name}; compare this fingerprint with the catalog author.",
            synced.revision.signer.key_id
        );
        if json_output {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    }
    Ok(synced)
}

fn print_sync_modules(name: &str, synced: &GitCatalogSync) {
    println!("Synced bucket {name} at commit {}.", synced.revision.commit);
    for report in &synced.modules.imported {
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
    for ignored in &synced.modules.ignored {
        let reason = match ignored.reason {
            ModuleIgnoreReason::UntrustedSource => "the bucket is not a trusted module source",
            ModuleIgnoreReason::OfficialPriority => "the official main bucket provides this game",
        };
        println!("Ignored game module {}: {reason}.", ignored.game_id);
    }
}

/// The pinned signer key ID that a trust acknowledgement must name.
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
            .git_revision(bucket)?
            .context("the bucket has no pinned signer")?
            .signer
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
    state_path: &Path,
    json_output: bool,
    game: Option<&str>,
) -> Result<()> {
    let root = state_path
        .parent()
        .context("invalid manager state directory")?;
    let store = BucketStore::new(root);
    if matches!(
        &command,
        BucketCommand::Add { .. }
            | BucketCommand::Remove { .. }
            | BucketCommand::Sync { .. }
            | BucketCommand::TrustModules { .. }
            | BucketCommand::UntrustModules { .. }
    ) {
        validate_registry_write(state_path, root, &store, game)?;
    }
    let value: Value = match command {
        BucketCommand::Sync {
            name,
            source,
            revision,
        } => {
            let source = source
                .zip(revision)
                .map(|(remote, reference)| GitCatalogSource::new(remote, reference));
            let synced = sync_catalog(&store, &name, source, json_output)?;
            if !json_output {
                print_sync_modules(&name, &synced);
            }
            serde_json::to_value(&synced)?
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
            return crate::bucket_update::run(name.as_deref(), json_output);
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
                        line
                    }
                }
            })
            .collect(),
    )
}

pub(super) fn search(
    query: Option<&str>,
    state_path: &Path,
    json_output: bool,
    game: Option<&str>,
) -> Result<()> {
    let root = state_path
        .parent()
        .context("invalid manager state directory")?;
    let catalog = BucketStore::new(root);
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
