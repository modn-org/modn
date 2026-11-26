//! The one game module that governs a command, with its origin and trust decision.

use super::loader::{self, VerifiedModule};
use super::official::read_official;
use super::registry::ModuleRegistry;
use crate::buckets::{decide_module_source_trust, BucketStore, ModuleSourceTrustDecision};
use crate::game_modules::ModuleSource;
use crate::manager_roots::ManagerRoots;
use std::fmt;
use std::path::{Path, PathBuf};

/// Where the governing game module came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ModuleOrigin {
    /// A development module directory selected for this command.
    DevelopmentOverride,
    /// The first-party module from the local official `main` bucket.
    OfficialBucket,
    /// The active retained release from the module registry.
    RetainedRelease,
}

/// The governing game module for one command.
///
/// A selection is built for the command, not cached: [`ModuleSelection::resolve`]
/// reads and verifies the module bytes on every call.
#[derive(Debug)]
pub struct ModuleSelection {
    origin: ModuleOrigin,
    module: VerifiedModule,
    script: String,
    trust: ModuleSourceTrustDecision,
}

impl ModuleSelection {
    /// Select the game module that governs one command using the fixed
    /// **governing-module precedence**.
    ///
    /// The precedence is:
    ///
    /// 1. **Development override**: [`ManagerRoots::development_modules`] as a
    ///    direct module directory, otherwise `MODN_GAME_MODULES_DIR/<game>` as
    ///    a base directory joined with the game.
    /// 2. **Official bucket**: the verified first-party module for the game when
    ///    the local official `main` bucket lists it.
    /// 3. **Retained release**: the active release in the module registry, and
    ///    only when its module-source trust decision is trusted.
    ///
    /// Otherwise resolution refuses with [`ModuleSelectionError::NoGameModule`].
    /// A refusal never falls through to another origin: a development override
    /// that fails verification and an untrusted retained release are refusals,
    /// not reasons to try the official module. A verified manifest whose
    /// `game_id` differs from the requested game is also refused.
    ///
    /// # Errors
    ///
    /// Returns [`ModuleSelectionError`] when the game ID is invalid, the selected
    /// module does not verify, the official or retained module cannot be read,
    /// the module identity differs, or no origin supplies the game.
    pub fn resolve(roots: &ManagerRoots, game: &str) -> Result<Self, ModuleSelectionError> {
        if !loader::valid_game_id(game) {
            return Err(ModuleSelectionError::InvalidGameId);
        }
        let selection = match development_directory(roots, game) {
            Some(directory) => development_override(&directory)?,
            None => without_override(roots, game)?,
        };
        if selection.module.game_id != game {
            return Err(ModuleSelectionError::IdentityMismatch {
                requested: game.to_owned(),
                module: selection.module.game_id.clone(),
            });
        }
        Ok(selection)
    }

    /// Where this module came from.
    #[must_use]
    pub fn origin(&self) -> ModuleOrigin {
        self.origin
    }

    /// The verified manifest and Lua entrypoint identity.
    #[must_use]
    pub fn module(&self) -> &VerifiedModule {
        &self.module
    }

    /// The verified Lua entrypoint source.
    #[must_use]
    pub fn script(&self) -> &str {
        &self.script
    }

    /// The trust decision that allowed this module to supply executable policy.
    #[must_use]
    pub fn trust(&self) -> &ModuleSourceTrustDecision {
        &self.trust
    }

    pub(crate) fn into_parts(self) -> (VerifiedModule, String) {
        (self.module, self.script)
    }
}

/// The explicit development override, when this command selects one.
fn development_directory(roots: &ManagerRoots, game: &str) -> Option<PathBuf> {
    roots
        .development_modules()
        .map(Path::to_path_buf)
        .or_else(|| {
            std::env::var_os("MODN_GAME_MODULES_DIR").map(|base| PathBuf::from(base).join(game))
        })
}

/// Use the development override and refuse if it does not verify.
fn development_override(directory: &Path) -> Result<ModuleSelection, ModuleSelectionError> {
    let (module, script) = loader::read_verified_module(directory)
        .map_err(|source| ModuleSelectionError::DevelopmentOverrideRejected { source })?;
    Ok(ModuleSelection {
        origin: ModuleOrigin::DevelopmentOverride,
        module,
        script,
        trust: development_trust(),
    })
}

/// Select the official module or a retained release, in that order.
fn without_override(
    roots: &ManagerRoots,
    game: &str,
) -> Result<ModuleSelection, ModuleSelectionError> {
    let store = BucketStore::new(roots);
    let official = store.official_games().map_err(unavailable)?;
    if official.iter().any(|id| id == game) {
        let (module, script) = read_official(roots, game).map_err(unavailable)?;
        return Ok(ModuleSelection {
            origin: ModuleOrigin::OfficialBucket,
            module,
            script,
            trust: official_trust(),
        });
    }
    let retained = ModuleRegistry::new(roots).read(game).map_err(unavailable)?;
    if let Some((module, script, registered)) = retained {
        let trust = store
            .module_source_trust(&registered.source)
            .map_err(unavailable)?;
        if !trust.is_trusted() {
            let bucket = registered
                .source
                .bucket
                .as_deref()
                .expect("a refused module source always names a bucket");
            return Err(ModuleSelectionError::UntrustedModuleSource {
                game: game.to_owned(),
                bucket: bucket.to_owned(),
                decision: Box::new(trust),
            });
        }
        return Ok(ModuleSelection {
            origin: ModuleOrigin::RetainedRelease,
            module,
            script,
            trust,
        });
    }
    no_game_module(roots, game)
}

/// Refuse a game that neither the official bucket nor the registry supplies.
///
/// The bucket store names the expected first-party path in its refusal, so the
/// refusal text is kept exactly as the store writes it.
fn no_game_module(
    roots: &ManagerRoots,
    game: &str,
) -> Result<ModuleSelection, ModuleSelectionError> {
    read_official(roots, game)
        .map(|(module, script)| ModuleSelection {
            origin: ModuleOrigin::OfficialBucket,
            module,
            script,
            trust: official_trust(),
        })
        .map_err(|error| ModuleSelectionError::NoGameModule {
            reason: error.to_string(),
        })
}

/// The trust decision for a directly selected development module.
///
/// A development override is selected by the developer and names no bucket, so
/// it carries the trusted decision of a module source without a bucket. There
/// is no bucket to distrust; the local selection is the acknowledgement.
fn development_trust() -> ModuleSourceTrustDecision {
    decide_module_source_trust(&ModuleSource::default(), None)
}

/// The trust decision for the official `main` module.
fn official_trust() -> ModuleSourceTrustDecision {
    decide_module_source_trust(
        &ModuleSource {
            bucket: Some("main".to_owned()),
            signer_key_id: None,
        },
        None,
    )
}

fn unavailable(source: anyhow::Error) -> ModuleSelectionError {
    ModuleSelectionError::Unavailable { source }
}

/// Why one command has no governing game module.
#[derive(Debug)]
#[non_exhaustive]
pub enum ModuleSelectionError {
    /// The requested game ID is not a valid game identifier.
    InvalidGameId,
    /// The development override does not read as a verified module.
    DevelopmentOverrideRejected {
        /// The verification failure, unchanged from the loader.
        source: anyhow::Error,
    },
    /// The verified manifest names a different game.
    IdentityMismatch {
        /// Game the command requested.
        requested: String,
        /// Game the verified manifest declares.
        module: String,
    },
    /// The retained release comes from a source that is not trusted.
    UntrustedModuleSource {
        /// Game the command requested.
        game: String,
        /// Bucket recorded by the retained release.
        bucket: String,
        /// The trust decision that refused the release.
        decision: Box<ModuleSourceTrustDecision>,
    },
    /// No official module and no retained release exist for the game.
    NoGameModule {
        /// The official module refusal, unchanged from the bucket store.
        reason: String,
    },
    /// A manager store could not be read, or an official module did not verify.
    Unavailable {
        /// The underlying failure.
        source: anyhow::Error,
    },
}

impl fmt::Display for ModuleSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidGameId => formatter.write_str("invalid game module identifier"),
            Self::DevelopmentOverrideRejected { source } => write!(formatter, "{source}"),
            Self::IdentityMismatch { .. } => {
                formatter.write_str("game module identity does not match requested game")
            }
            Self::UntrustedModuleSource {
                game,
                bucket,
                decision,
            } => {
                let remedy = decision
                    .remedy()
                    .expect("a refused module source always names a remedy");
                write!(
                    formatter,
                    "game module {game} comes from bucket {bucket}, which is not a trusted module source; run `{}` to trust it again, or `{}` to remove the module",
                    remedy.trust_modules_command(),
                    remedy.remove_module_command(game)
                )
            }
            Self::NoGameModule { reason } => formatter.write_str(reason),
            Self::Unavailable { source } => write!(formatter, "{source}"),
        }
    }
}

impl std::error::Error for ModuleSelectionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::DevelopmentOverrideRejected { source } | Self::Unavailable { source } => {
                Some(source.root_cause())
            }
            _ => None,
        }
    }
}
