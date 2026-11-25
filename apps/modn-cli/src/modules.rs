use crate::commands::ModuleCommand;
use crate::state_access::validate_write_targets;
use anyhow::Result;
use modn_core::game_modules::{check_module_with_roots, ModuleRegistry, ModuleTrust};
use modn_core::manager_roots::ManagerRoots;
use std::path::PathBuf;

pub(super) fn content_root_appdata(game_id: &str) -> Result<Option<PathBuf>> {
    if modn_core::game_modules::content_root_spec(game_id)?.is_some() {
        Ok(Some(modn_core::content_root::appdata_directory()?))
    } else {
        Ok(None)
    }
}

pub(super) fn run(command: ModuleCommand, roots: &ManagerRoots, json_output: bool) -> Result<()> {
    let data = roots.profile();
    let registry = ModuleRegistry::new(data);
    if matches!(
        &command,
        ModuleCommand::Pin { .. }
            | ModuleCommand::Unpin { .. }
            | ModuleCommand::Update { .. }
            | ModuleCommand::Rollback { .. }
    ) {
        return crate::module_releases::run(command, roots, json_output);
    }
    if matches!(
        command,
        ModuleCommand::Register { trust: true, .. } | ModuleCommand::Remove { .. }
    ) {
        validate_write_targets(roots, None)?;
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
        ModuleCommand::List => {
            return crate::module_releases::list(roots, json_output);
        }
        ModuleCommand::Remove { game } => {
            registry.unregister(&game)?;
            serde_json::json!({"removed": game})
        }
        ModuleCommand::Pin { .. }
        | ModuleCommand::Unpin { .. }
        | ModuleCommand::Update { .. }
        | ModuleCommand::Rollback { .. } => {
            unreachable!("release commands return before registry lookup")
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
    if let Some(digest) = value.get("blake3").and_then(serde_json::Value::as_str) {
        println!("BLAKE3: {digest}");
    }
}
