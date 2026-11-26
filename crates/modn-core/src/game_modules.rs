mod capabilities;
mod import;
mod launch_data;
mod loader;
mod official;
pub(crate) mod policy;
mod registry;
mod resolver;
mod runtime;
mod selection;
mod store_data;
mod visibility;

#[cfg(test)]
pub(crate) use import::FAIL_NEXT_CATALOG_COMMIT;
pub(crate) use import::{ignored_catalog_modules, stage_catalog_modules_at};
pub use import::{
    stage_catalog_modules, CatalogModuleReport, IgnoredModule, ModuleIgnoreReason,
    StagedCatalogImport,
};
pub use launch_data::LaunchTargets;
pub use loader::{
    check_module, check_module_with_roots, ContentRootSpec, GameModuleInfo, VerifiedModule,
    HOST_API_VERSION, MIN_HOST_API_VERSION,
};
pub use registry::{
    ActivationRepair, ImportReport, ModulePin, ModuleRegistry, ModuleSource, ModuleTrust,
    RegisteredModule, RegistryOpen, ReleaseOutcome,
};
pub use resolver::available_games;
pub use selection::{ModuleOrigin, ModuleSelection, ModuleSelectionError};
pub use store_data::StoreIdentities;
pub use visibility::{hidden_modules, HiddenModule};

/// Read the selected module's declared content root without executing it.
pub fn content_root_spec(selection: &ModuleSelection) -> anyhow::Result<Option<ContentRootSpec>> {
    selection.verify()?;
    Ok(selection.module().content_root().cloned())
}

/// Read the selected module's declared store identities without executing it.
pub fn store_identities(selection: &ModuleSelection) -> anyhow::Result<StoreIdentities> {
    selection.verify()?;
    Ok(selection.module().stores().clone())
}

/// Read the selected module's declared launch targets without executing it.
pub fn launch_targets(selection: &ModuleSelection) -> anyhow::Result<LaunchTargets> {
    selection.verify()?;
    Ok(selection.module().launch().clone())
}
