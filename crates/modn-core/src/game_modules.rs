mod capabilities;
mod import;
mod launch_data;
mod loader;
mod official;
pub(crate) mod policy;
mod registry;
mod resolver;
mod runtime;
mod store_data;
mod visibility;

pub(crate) use import::{ignored_catalog_modules, stage_catalog_modules_at};
pub use import::{
    stage_catalog_modules, CatalogModuleReport, IgnoredModule, ModuleIgnoreReason,
    StagedCatalogImport,
};
pub use launch_data::LaunchTargets;
pub use loader::{
    check_module, check_module_with_roots, ContentRootSpec, GameModuleInfo, HOST_API_VERSION,
    MIN_HOST_API_VERSION,
};
pub use registry::{
    ImportReport, ModulePin, ModuleRegistry, ModuleSource, ModuleTrust, RegisteredModule,
    ReleaseOutcome,
};
pub use resolver::{available_games, with_module_source};
pub use store_data::StoreIdentities;
pub use visibility::{hidden_modules, HiddenModule};

/// Read the selected module's declared content root without executing it.
pub fn content_root_spec(game_id: &str) -> anyhow::Result<Option<ContentRootSpec>> {
    Ok(resolver::read_policy(game_id)?.0.content_root)
}

/// Read the selected module's declared store identities without executing it.
pub fn store_identities(game_id: &str) -> anyhow::Result<StoreIdentities> {
    Ok(resolver::read_policy(game_id)?.0.stores)
}

/// Read the selected module's declared launch targets without executing it.
pub fn launch_targets(game_id: &str) -> anyhow::Result<LaunchTargets> {
    Ok(resolver::read_policy(game_id)?.0.launch)
}
