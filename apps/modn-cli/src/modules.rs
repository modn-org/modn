//! Module distribution never opens writable game registration or ownership state.
use crate::commands::ModuleCommand;
use crate::state_access::validate_state_path;
use anyhow::{bail, Context, Result};
use modn_core::game_modules::{
    check_module_with_roots, inspect_package, module_origin, ModulePackage, ModuleStore,
};
use modn_core::state::Store;
use serde_json::{json, Value};
use std::path::Path;

pub(super) fn run(
    command: ModuleCommand,
    state_path: &Path,
    requested: Option<&str>,
    json_output: bool,
) -> Result<()> {
    let store = ModuleStore::new(
        state_path
            .parent()
            .context("invalid module state directory")?,
    );
    let result: Value = match command {
        ModuleCommand::Check {
            directory,
            allow_roots,
        } => {
            let info = check_module_with_roots(&directory, &allow_roots)?;
            if !json_output {
                println!(
                    "Game module {} v{} (host API {})",
                    info.game_id, info.version, info.api_version
                );
                return Ok(());
            }
            serde_json::to_value(info)?
        }
        ModuleCommand::Inspect { archive } => serde_json::to_value(inspect_package(&archive)?)?,
        ModuleCommand::Install {
            archive,
            dry_run,
            yes,
        } => {
            if !dry_run && !yes {
                bail!("module install requires --dry-run or --yes; installation does not enable the unsigned module");
            }
            let package = ModulePackage::read(&archive)?;
            match_game(requested, &package.info().game_id)?;
            if !dry_run {
                validate_write(state_path, &package.info().game_id)?;
                package
                    .ensure_outside_game(state_path.parent().context("invalid state directory")?)?;
                warn_unsigned();
                serde_json::to_value(store.install(&package)?)?
            } else {
                store.check_install(&package)?;
                serde_json::to_value(package.info())?
            }
        }
        ModuleCommand::Update { archive, yes } => {
            approve(yes)?;
            let package = ModulePackage::read(&archive)?;
            match_game(requested, &package.info().game_id)?;
            validate_write(state_path, &package.info().game_id)?;
            package.ensure_outside_game(state_path.parent().context("invalid state directory")?)?;
            warn_unsigned();
            serde_json::to_value(store.update(&package)?)?
        }
        ModuleCommand::Enable { game, version, yes } => {
            approve(yes)?;
            match_game(requested, &game)?;
            validate_write(state_path, &game)?;
            warn_unsigned();
            store.enable(&game, &version)?;
            json!({"game_id":game,"version":version,"enabled":true})
        }
        ModuleCommand::Disable { game, yes } => {
            approve(yes)?;
            match_game(requested, &game)?;
            validate_write(state_path, &game)?;
            store.disable(&game)?;
            json!({"game_id":game,"enabled":false})
        }
        ModuleCommand::Rollback { game, yes } => {
            approve(yes)?;
            match_game(requested, &game)?;
            validate_write(state_path, &game)?;
            store.rollback(&game)?;
            serde_json::to_value(store.status(&game)?)?
        }
        ModuleCommand::List => serde_json::to_value(store.list()?)?,
        ModuleCommand::Status { game } => {
            match_game(requested, &game)?;
            let mut status = serde_json::to_value(store.status(&game)?)?;
            status["effective_source"] = match module_origin(&game) {
                Ok(origin) => json!({"origin":origin}),
                Err(error) => json!({"origin":"unavailable", "error":format!("{error:#}")}),
            };
            status
        }
    };
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn approve(yes: bool) -> Result<()> {
    if !yes {
        bail!("module changes require explicit --yes");
    }
    Ok(())
}

fn match_game(requested: Option<&str>, game: &str) -> Result<()> {
    if requested.is_some_and(|id| id != game) {
        bail!("--game does not match module game ID");
    }
    Ok(())
}

fn validate_write(state_path: &Path, game: &str) -> Result<()> {
    let roots = if state_path.is_file() {
        Store::open_readonly(state_path)?
            .list()?
            .into_iter()
            .map(|entry| entry.path)
            .collect()
    } else {
        Vec::new()
    };
    validate_state_path(state_path, Some(game), None, &roots)
}

fn warn_unsigned() {
    eprintln!("warning: unsigned local Lua module; SHA-256 verifies integrity, not publisher identity or safety. Enabled modules execute bounded sandbox logic and can propose game-file mappings.");
}
