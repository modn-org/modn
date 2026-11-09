use crate::state_access::ManagerState;
use anyhow::Result;
use modn_core::history::HistoryOperation;
use std::path::Path;

fn games(operation: &HistoryOperation) -> String {
    operation
        .packages
        .iter()
        .map(|package| package.mod_id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn run(state_path: &Path, game_id: Option<&str>, json: bool) -> Result<()> {
    let operations = ManagerState::load_deferred(state_path)?
        .into_existing()
        .map(|store| store.history(game_id))
        .transpose()?
        .unwrap_or_default();
    if json {
        println!("{}", serde_json::to_string_pretty(&operations)?);
        return Ok(());
    }
    if operations.is_empty() {
        println!("No committed operations recorded.");
        return Ok(());
    }
    let id_width = operations
        .iter()
        .map(|operation| operation.id.len())
        .max()
        .unwrap_or(2)
        .max("ID".len());
    let game_width = operations
        .iter()
        .map(|operation| operation.game_id.len())
        .max()
        .unwrap_or(4)
        .max("GAME".len());
    println!(
        "{:<id_width$}  {:<9}  {:<game_width$}  {:<9}  RECORDED",
        "ID", "KIND", "GAME", "PACKAGES"
    );
    for operation in &operations {
        println!(
            "{:<id_width$}  {:<9}  {:<game_width$}  {:<9}  {}",
            operation.id,
            operation.kind.as_str(),
            operation.game_id,
            games(operation),
            operation.recorded_at
        );
    }
    Ok(())
}
