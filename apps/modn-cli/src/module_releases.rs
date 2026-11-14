use crate::commands::ModuleCommand;
use anyhow::{bail, Context, Result};
use modn_core::buckets::{local_bucket_module_refusal, BucketStore, ModuleTrustState};
use modn_core::game_modules::{ModuleRegistry, ModuleSource, ReleaseOutcome};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

/// Keep one release, fetch a newer one, or move the active release pointer.
pub(super) fn run(command: ModuleCommand, state_path: &Path, json_output: bool) -> Result<()> {
    let data = state_path
        .parent()
        .context("invalid manager state directory")?;
    crate::modules::validate_registry_writes(state_path, data)?;
    let registry = ModuleRegistry::new(data);
    match command {
        ModuleCommand::Pin { game } => {
            let pin = registry.pin(&game)?;
            if json_output {
                print_json(&json!({
                    "game_id": game,
                    "pinned": {
                        "bucket": pin.bucket,
                        "version": pin.version,
                        "sha256": pin.sha256,
                    },
                }))?;
            } else {
                println!(
                    "Pinned game module {game} at version {}; automatic updates and rollback will not move it.",
                    pin.version
                );
            }
        }
        ModuleCommand::Unpin { game } => {
            registry.unpin(&game)?;
            if json_output {
                print_json(&json!({"game_id": game, "pinned": null}))?;
            } else {
                println!("Unpinned game module {game}; updates and rollback may move it again.");
            }
        }
        ModuleCommand::Update { game } => update(&registry, data, &game, json_output)?,
        ModuleCommand::Rollback { game, to } => {
            rollback(&registry, &game, to.as_deref(), json_output)?;
        }
        ModuleCommand::Register { .. }
        | ModuleCommand::List
        | ModuleCommand::Remove { .. }
        | ModuleCommand::Check { .. } => {
            unreachable!("only release commands reach module_releases::run")
        }
    }
    Ok(())
}

fn update(
    registry: &ModuleRegistry,
    store_root: &Path,
    game: &str,
    json_output: bool,
) -> Result<()> {
    let active = registry
        .active(game)?
        .with_context(|| format!("no registered game module for {game}"))?;
    let from = active.module.version.clone();
    let Some(bucket) = active.source.bucket.clone() else {
        bail!(
            "game module {game} was registered from a local source; register the new version with `modn module register <directory> --trust`"
        );
    };
    let store = BucketStore::new(store_root);
    match store.module_trust(&bucket)? {
        ModuleTrustState::Trusted | ModuleTrustState::Main => {}
        ModuleTrustState::Untrusted => bail!(
            "game module {game} comes from bucket {bucket}, which is not a trusted module source; run `modn bucket trust-modules {bucket}` to trust it again, or `modn module remove {game}` to remove the module"
        ),
        ModuleTrustState::Local => {
            bail!("{}", local_bucket_module_refusal(&bucket))
        }
        _ => bail!("unsupported module trust state for bucket {bucket}"),
    }
    let pinned = registry.pin_status(game)?.is_some();
    let synced = store.sync_git_with_signer(&bucket, None)?;
    let report = synced
        .modules
        .imported
        .iter()
        .find(|report| report.game_id == game);
    let to = registry
        .active(game)?
        .map(|release| release.module.version)
        .unwrap_or_else(|| from.clone());
    let outcome = if pinned {
        ReleaseOutcome::Pinned
    } else {
        report.map_or(ReleaseOutcome::Unchanged, |report| report.outcome)
    };
    if json_output {
        print_json(&json!({
            "game_id": game,
            "from": from,
            "to": to,
            "outcome": outcome,
        }))?;
    } else if outcome == ReleaseOutcome::Pinned {
        println!(
            "Game module {game} is pinned at {from}; run `modn module unpin {game}` to allow updates."
        );
    } else {
        println!(
            "Game module {game}: {from} -> {to} ({}).",
            outcome_name(outcome)
        );
    }
    Ok(())
}

fn rollback(
    registry: &ModuleRegistry,
    game: &str,
    to: Option<&str>,
    json_output: bool,
) -> Result<()> {
    let active = registry
        .active(game)?
        .with_context(|| format!("no registered game module for {game}"))?;
    let from = active.module.version;
    if let Some(pin) = registry.pin_status(game)? {
        bail!(
            "game module {game} is pinned at {}; run `modn module unpin {game}` before rollback",
            pin.version
        );
    }
    let target = registry.rollback(game, to)?;
    let to = target.module.version;
    if json_output {
        print_json(&json!({"game_id": game, "from": from, "to": to}))?;
    } else {
        println!(
            "Rolled back game module {game} from {from} to {to}; no installed files or game files changed."
        );
    }
    Ok(())
}

/// List every retained release and mark its active, pin and trust state.
pub(super) fn list(data: &Path, state_path: &Path, json_output: bool) -> Result<()> {
    let registry = ModuleRegistry::new(data);
    let store_root = state_path
        .parent()
        .context("invalid manager state directory")?;
    let store = BucketStore::new(store_root);
    let mut trust = BTreeMap::new();
    let mut entries = Vec::new();
    let mut lines = Vec::new();
    for active in registry.list()? {
        let game = active.module.game_id;
        let pin = registry.pin_status(&game)?;
        for release in registry.releases(&game)? {
            let source_trusted = source_trusted(&store, &release.source, &mut trust);
            let pinned = pin.as_ref().is_some_and(|pin| {
                pin.version == release.module.version && pin.sha256 == release.sha256
            });
            if json_output {
                entries.push(json!({
                    "module": release.module,
                    "sha256": release.sha256,
                    "source": release.source,
                    "active": release.active,
                    "source_trusted": source_trusted,
                    "pinned": pinned,
                }));
                continue;
            }
            let mut markers = Vec::new();
            if release.active {
                markers.push("active");
            }
            if pinned {
                markers.push("pinned");
            }
            let suffix = if markers.is_empty() {
                String::new()
            } else {
                format!(" [{}]", markers.join(", "))
            };
            lines.push(format!(
                "Game module {game} v{} (host API {}){suffix} {}",
                release.module.version,
                release.module.api_version,
                describe_source(&release.source)
            ));
            lines.push(format!("SHA-256: {}", release.sha256));
            if !source_trusted {
                if let Some(bucket) = release.source.bucket.as_deref() {
                    lines.push(format!(
                        "warning: bucket {bucket} is not a trusted module source; this release is refused at use. Run `modn bucket trust-modules {bucket}` or `modn module remove {game}`."
                    ));
                }
            }
        }
    }
    if json_output {
        print_json(&Value::Array(entries))?;
    } else {
        for line in lines {
            println!("{line}");
        }
    }
    Ok(())
}

fn describe_source(source: &ModuleSource) -> String {
    match source.bucket.as_deref() {
        Some(bucket) => match source.signer_key_id.as_deref() {
            Some(signer) => format!("from bucket {bucket} (signer {signer})"),
            None => format!("from bucket {bucket}"),
        },
        None => "from local registration".to_owned(),
    }
}

fn source_trusted(
    store: &BucketStore,
    source: &ModuleSource,
    cache: &mut BTreeMap<String, bool>,
) -> bool {
    let Some(bucket) = source.bucket.as_deref() else {
        return true;
    };
    if let Some(trusted) = cache.get(bucket) {
        return *trusted;
    }
    let trusted = matches!(
        store.module_trust(bucket),
        Ok(ModuleTrustState::Main | ModuleTrustState::Trusted)
    );
    cache.insert(bucket.to_owned(), trusted);
    trusted
}

fn outcome_name(outcome: ReleaseOutcome) -> &'static str {
    match outcome {
        ReleaseOutcome::Imported => "imported",
        ReleaseOutcome::Unchanged => "unchanged",
        ReleaseOutcome::NotNewer => "not_newer",
        ReleaseOutcome::Pinned => "pinned",
    }
}

fn print_json(value: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
