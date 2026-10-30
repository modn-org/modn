//! Versioned game-module loader and read-only host capability boundary.
mod capabilities;
mod loader;
mod package;
pub(crate) mod policy;
mod resolver;
mod runtime;
mod store;
mod store_files;

pub use loader::{check_module, check_module_with_roots, GameModuleInfo, HOST_API_VERSION};
pub use package::{inspect_package, ModulePackage, PackageInfo};
pub use resolver::{module_origin, with_module_store};
pub use store::{ModuleStatus, ModuleStore};
