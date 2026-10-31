//! First-party Lua game-module loader and read-only host capability boundary.
mod capabilities;
mod loader;
pub(crate) mod policy;
mod resolver;
mod runtime;

pub use loader::{check_module, check_module_with_roots, GameModuleInfo, HOST_API_VERSION};
pub use resolver::with_module_source;
