use crate::commands::BucketCommand;
use crate::state_access::{validate_state_path, ManagerState};
use anyhow::{Context, Result};
use modn_core::buckets::{BucketStore, GitCatalogSource};
use modn_core::state::Store;
use serde_json::Value;
use std::path::Path;

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
        BucketCommand::Add { .. } | BucketCommand::Remove { .. } | BucketCommand::Sync { .. }
    ) {
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
            serde_json::to_value(store.sync_git(&name, source)?)?
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
                    _ => format!(
                        "{prefix}: {}",
                        entry.get("title").and_then(Value::as_str).unwrap_or("")
                    ),
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
