use crate::commands::BucketCommand;
use anyhow::{Context, Result};
use modn_core::buckets::BucketStore;
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
    let value: Value = match command {
        BucketCommand::Add { name, path } => serde_json::to_value(store.add(&name, &path)?)?,
        BucketCommand::Remove { name } => {
            store.remove(&name)?;
            serde_json::json!({"removed": name})
        }
        BucketCommand::List => serde_json::to_value(store.list()?)?,
        BucketCommand::Search => serde_json::to_value(store.packages(game)?)?,
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
                match (entry.get("game"), entry.get("id"), entry.get("version")) {
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
