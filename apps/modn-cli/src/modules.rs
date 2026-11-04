use crate::commands::ModuleCommand;
use crate::state_access::{validate_state_path, ManagerState};
use anyhow::Result;
use modn_core::buckets::BucketStore;
use modn_core::data_dir;
use modn_core::game_modules::{check_module_with_roots, ModuleRegistry, ModuleTrust};
use modn_core::state::Store;
use std::path::Path;

pub(super) fn run(command: ModuleCommand, state_path: &Path, json_output: bool) -> Result<()> {
    let data = data_dir::data_directory()?;
    let registry = ModuleRegistry::new(&data);
    if matches!(
        command,
        ModuleCommand::Register { trust: true, .. } | ModuleCommand::Remove { .. }
    ) {
        let state = ManagerState::load_deferred(state_path)?;
        let roots = state
            .existing()
            .map(Store::list)
            .transpose()?
            .unwrap_or_default()
            .into_iter()
            .map(|entry| entry.path)
            .collect::<Vec<_>>();
        validate_state_path(&data.join("game-modules/registry.lock"), None, None, &roots)?;
        for game in BucketStore::new(&data).official_games()? {
            validate_state_path(
                &data.join("game-modules/registry.lock"),
                Some(&game),
                None,
                &roots,
            )?;
        }
    }
    let value = match command {
        ModuleCommand::Check {
            directory,
            allow_roots,
        } => serde_json::to_value(check_module_with_roots(&directory, &allow_roots)?)?,
        ModuleCommand::Register { directory, trust } => {
            let trust = if trust {
                ModuleTrust::Acknowledged
            } else {
                ModuleTrust::NotAcknowledged
            };
            serde_json::to_value(registry.register(&directory, trust)?)?
        }
        ModuleCommand::List => serde_json::to_value(registry.list()?)?,
        ModuleCommand::Remove { game } => {
            registry.unregister(&game)?;
            serde_json::json!({"removed": game})
        }
    };
    if json_output {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else if let Some(game) = value.get("removed").and_then(serde_json::Value::as_str) {
        println!("Removed trusted module {game}");
    } else if let Some(entries) = value.as_array() {
        for entry in entries {
            print_module(entry);
        }
    } else {
        print_module(&value);
    }
    Ok(())
}

fn print_module(value: &serde_json::Value) {
    let module = value.get("module").unwrap_or(value);
    println!(
        "Game module {} v{} (host API {})",
        module["game_id"].as_str().unwrap_or(""),
        module["version"].as_str().unwrap_or(""),
        module["api_version"]
    );
    if let Some(digest) = value.get("sha256").and_then(serde_json::Value::as_str) {
        println!("SHA-256: {digest}");
    }
}
