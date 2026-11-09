mod capabilities;
mod loader;
mod official;
pub(crate) mod policy;
mod registry;
mod resolver;
mod runtime;

pub use loader::{
    check_module, check_module_with_roots, ContentRootSpec, GameModuleInfo, HOST_API_VERSION,
    MIN_HOST_API_VERSION,
};
pub use registry::{
    ImportReport, ModulePin, ModuleRegistry, ModuleSource, ModuleTrust, RegisteredModule,
    ReleaseOutcome,
};
pub use resolver::with_module_source;

/// Read the selected module's declared content root without executing it.
pub fn content_root_spec(game_id: &str) -> anyhow::Result<Option<ContentRootSpec>> {
    Ok(resolver::read_policy(game_id)?.0.content_root)
}
