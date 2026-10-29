//! Versioned game-module loader and read-only host capability boundary.
mod capabilities;
mod loader;

pub use loader::{check_module, check_module_with_roots, GameModuleInfo, HOST_API_VERSION};
