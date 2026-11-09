use crate::game_modules::policy;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Discovered installation and the source of its location.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_structs,
    reason = "CLI constructs explicit manual game candidates"
)]
pub struct Candidate {
    /// Canonical installation directory.
    pub path: PathBuf,
    /// Registration origin: `steam`, `gog`, `epic` or `manual`.
    pub source: String,
}

/// Validate an installation against a runtime-selected game's Lua policy.
pub fn validate_installation_for(game_id: &str, path: &Path) -> Result<PathBuf> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("cannot access game directory {}", path.display()))?;
    let (valid, marker) = policy::validate(game_id, &canonical)?;
    if !valid {
        bail!("{} does not contain {}", canonical.display(), marker);
    }
    Ok(canonical)
}

/// Recognize a candidate without turning Lua policy failures into "not a game".
///
/// Use for manager-state write boundaries: a missing or broken selected policy
/// must fail closed, while a valid policy may return false for other directories.
pub fn recognizes_installation_for(game_id: &str, path: &Path) -> Result<bool> {
    let canonical = path.canonicalize()?;
    Ok(policy::validate(game_id, &canonical)?.0)
}
