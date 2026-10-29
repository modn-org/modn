//! Versioned game-module loader and read-only host capability boundary.
mod capabilities;
mod loader;
pub(crate) mod policy;
mod runtime;

pub use loader::{check_module, check_module_with_roots, GameModuleInfo, HOST_API_VERSION};
