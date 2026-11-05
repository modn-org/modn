mod capabilities;
mod loader;
mod official;
pub(crate) mod policy;
mod registry;
mod resolver;
mod runtime;

pub use loader::{check_module, check_module_with_roots, GameModuleInfo, HOST_API_VERSION};
pub use registry::{ModuleRegistry, ModuleTrust, RegisteredModule};
pub use resolver::with_module_source;
