//! Shared admission of one candidate game-module release.
//!
//! Single-source registration and catalog import both validate a candidate and
//! hand it to [`admit`]. That routine computes the digest, refuses a version
//! that is already retained with another digest, honours the pin, stages a
//! checked copy and compares versions. Activation stays with the callers.

use super::super::loader::{self, VerifiedModule};
use super::{layout, ModulePin, ModuleSource};
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;

/// Retained registry state an admission compares a candidate against.
pub(in crate::game_modules) struct RegistryState {
    current: Option<String>,
    pinned: bool,
}

impl RegistryState {
    /// Read the active release and pin of one game without changing storage.
    pub(in crate::game_modules) fn read(directory: &Path, game: &str) -> Result<Self> {
        let current = layout::open_game(directory, game)?;
        if let Some(active) = current.as_deref() {
            layout::read_release(directory, game, active)?;
        }
        let pinned = ModulePin::read(directory, game)?.is_some();
        Ok(Self { current, pinned })
    }
}

/// One validated candidate release and the state it must be admitted against.
pub(in crate::game_modules) struct Candidate<'a> {
    /// Registry directory of the candidate's game.
    pub(in crate::game_modules) directory: &'a Path,
    /// Validated manifest identity and policy.
    pub(in crate::game_modules) manifest: &'a VerifiedModule,
    /// Entrypoint Lua source.
    pub(in crate::game_modules) script: &'a str,
    /// Exact manifest bytes that the digest covers.
    pub(in crate::game_modules) manifest_bytes: &'a [u8],
    /// Bucket and signer that the receipt records.
    pub(in crate::game_modules) source: &'a ModuleSource,
    /// Retained state read by [`RegistryState::read`] before admission.
    pub(in crate::game_modules) state: RegistryState,
    /// Directory that receives the staged copy; it must not exist yet.
    pub(in crate::game_modules) staged: &'a Path,
}

/// What admission did with one candidate.
pub(in crate::game_modules) enum Admission {
    /// The release is staged and the caller may activate it.
    Admitted(AdmittedRelease),
    /// The same release is already retained, so nothing was written.
    Unchanged {
        /// BLAKE3 digest of the retained and incoming bytes.
        blake3: String,
    },
    /// A pin refused the release, so nothing was written.
    Refused {
        /// BLAKE3 digest of the refused bytes.
        blake3: String,
    },
}

/// Digest and activation facts of one admitted release.
pub(in crate::game_modules) struct AdmittedRelease {
    /// BLAKE3 digest of the manifest bytes followed by the entrypoint bytes.
    pub(in crate::game_modules) blake3: String,
    /// Version that was active before admission, if any.
    pub(in crate::game_modules) current: Option<String>,
    /// Whether the candidate sorts above the previously active release.
    pub(in crate::game_modules) newer: bool,
}

/// Admit one candidate release into the registry's staging area.
///
/// The staged copy carries the manifest, the entrypoint and the receipt. The
/// routine never moves the copy into `versions` and never writes `current`.
pub(in crate::game_modules) fn admit(candidate: Candidate<'_>) -> Result<Admission> {
    let Candidate {
        directory,
        manifest,
        script,
        manifest_bytes,
        source,
        state,
        staged,
    } = candidate;
    let game = manifest.game_id.as_str();
    let blake3 = layout::digest(manifest_bytes, script.as_bytes());
    let version_directory = layout::release_directory(directory, &manifest.version);
    match fs::symlink_metadata(&version_directory) {
        Ok(_) => {
            let existing = layout::read_release(directory, game, &manifest.version)?;
            if existing.blake3 == blake3 {
                return Ok(Admission::Unchanged { blake3 });
            }
            bail!(
                "module {game} version {} is already retained with a different digest (existing source: {}; incoming source: {}); remove and register the module again",
                manifest.version,
                existing.source.describe(),
                source.describe()
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if state.pinned {
        return Ok(Admission::Refused { blake3 });
    }
    fs::create_dir(staged)?;
    fs::write(staged.join("module.json"), manifest_bytes)?;
    fs::write(staged.join(&manifest.entrypoint), script.as_bytes())?;
    let checked = loader::check_module(staged).with_context(|| {
        format!(
            "game module {game} version {} failed staging validation",
            manifest.version
        )
    })?;
    if checked.game_id != manifest.game_id
        || checked.version != manifest.version
        || checked.api_version != manifest.api_version
    {
        bail!(
            "game module {game} version {} identity changed during staging",
            manifest.version
        );
    }
    let receipt = layout::Receipt {
        game_id: checked.game_id,
        version: checked.version,
        api_version: checked.api_version,
        blake3: blake3.clone(),
        bucket: source.bucket.clone(),
        signer_key_id: source.signer_key_id.clone(),
    };
    fs::write(staged.join(layout::RECEIPT), serde_json::to_vec(&receipt)?)?;
    let newer = state
        .current
        .as_deref()
        .is_none_or(|active| layout::compare_versions(&manifest.version, active).is_gt());
    Ok(Admission::Admitted(AdmittedRelease {
        blake3,
        current: state.current,
        newer,
    }))
}
