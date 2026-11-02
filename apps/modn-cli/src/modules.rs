use crate::commands::ModuleCommand;
use anyhow::Result;
use modn_core::game_modules::check_module_with_roots;

pub(super) fn run(command: ModuleCommand, json_output: bool) -> Result<()> {
    let ModuleCommand::Check {
        directory,
        allow_roots,
    } = command;
    let info = check_module_with_roots(&directory, &allow_roots)?;
    if json_output {
        println!("{}", serde_json::to_string_pretty(&info)?);
    } else {
        println!(
            "Game module {} v{} (host API {})",
            info.game_id, info.version, info.api_version
        );
    }
    Ok(())
}
