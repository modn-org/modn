use crate::commands::BucketCommand;
use crate::state_access::validate_state_path;
use anyhow::{Context, Result};
use modn_core::buckets::BucketStore;
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
        BucketCommand::Add { .. } | BucketCommand::Remove { .. }
    ) {
        // Keep catalog mutations outside known game roots, even when the
        // operator supplies --state-dir without --game. Game identities come
        // from data and registered state, not game-specific Rust adapters.
        let existing = if state_path.is_file() {
            Some(Store::open_readonly(state_path)?)
        } else {
            None
        };
        let registered = existing
            .as_ref()
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
        // Validate the actual registry write location, not just its state
        // parent: a game may itself reside in <state-dir>/buckets.
        let registry_target = root.join("buckets/registrations/.bucket-entry.json");
        if games.is_empty() {
            validate_state_path(&registry_target, None, None, &roots)?;
        }
        for game in games {
            validate_state_path(&registry_target, Some(&game), None, &roots)?;
        }
    }
    let value: Value = match command {
        BucketCommand::Add { name, path } => serde_json::to_value(store.add(&name, &path)?)?,
        BucketCommand::Remove { name } => {
            store.remove(&name)?;
            serde_json::json!({"removed": name})
        }
        BucketCommand::List => serde_json::to_value(store.list()?)?,
        BucketCommand::Search => serde_json::to_value(store.packages(game)?)?,
        BucketCommand::Modules => serde_json::to_value(store.modules(game)?)?,
        BucketCommand::ModuleShow { module } => serde_json::to_value(store.show_module(&module)?)?,
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
                let id = entry.get("id").or_else(|| entry.get("version"));
                match (game, id, entry.get("version")) {
                    (Some(game), Some(id), Some(version)) => format!(
                        "{prefix}/{}/{}, version {}",
                        game.as_str().unwrap_or(""),
                        id.as_str().unwrap_or(""),
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
