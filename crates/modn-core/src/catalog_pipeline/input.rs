//! The data one catalog install plan reads, except prompts and output.

use crate::acquisition::{ArchiveCache, CacheUse};
use crate::approval::Capabilities;
use crate::buckets::{BucketPackage, BucketSnapshot};
use crate::state::Store;
use std::path::{Path, PathBuf};

/// Everything one catalog install needs, except prompts and output.
///
/// Every field is data or a durable read. The value grants nothing: the core
/// never prompts and never derives a consent value from it.
#[derive(Clone)]
#[non_exhaustive]
pub struct InstallInput<'a> {
    /// The catalog release the caller selected after ambiguity resolution.
    pub selected: &'a BucketPackage,
    /// The one snapshot this command uses for pins, conflicts and versions.
    pub catalog: &'a BucketSnapshot,
    /// Read-only manager state, when one exists.
    pub store: Option<&'a Store>,
    /// The game and installation that the plan targets.
    pub target: InstallTarget<'a>,
    /// The command's `--file` values, not yet matched or verified.
    pub supplied: &'a [PathBuf],
    /// The manager profile archive cache.
    pub cache: &'a ArchiveCache,
    /// Whether a verified cache entry may serve a release.
    pub cache_use: CacheUse,
    /// The command-line grants, as data.
    pub consent: ConsentInputs,
}

/// The game and installation that one plan targets.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct InstallTarget<'a> {
    /// Game identifier of the selected release.
    pub game_id: &'a str,
    /// Canonical game installation root.
    pub installation: &'a Path,
}

/// The command-line grants, as data. The core never prompts and never infers
/// one.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct ConsentInputs {
    /// `--allow-native-injector` and `--allow-executable` folded into one set.
    pub flags: Capabilities,
}
