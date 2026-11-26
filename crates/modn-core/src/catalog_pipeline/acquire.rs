//! Acquire the archives that one catalog plan lacks.
//!
//! Every missing release is acquired through
//! [`ArchiveCache::acquire_release`], the one verified acquisition routine. The
//! function is pure with respect to the game and the manager state: it may
//! write the verified archive cache, and it reports progress and cancellation
//! through the supplied [`OperationControl`]. It never prompts and never
//! prints.

use super::plan::CatalogPlan;
use crate::acquisition::{
    ArchiveCache, CacheUse, ReleaseDigest, ReleaseResolver, SourceAttempt, SourcePolicy,
    SourceTransport,
};
use crate::buckets::PackageSource;
use crate::error::{coded, ErrorCode};
use crate::progress::OperationControl;
use crate::transactions::CatalogReference;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// One planned release whose archive the acquisition obtained.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct AcquiredArchive {
    /// The release the archive serves.
    pub reference: CatalogReference,
    /// Verified archive file, in the cache.
    pub archive: PathBuf,
    /// Digest the acquisition verified.
    pub digest: ReleaseDigest,
    /// Manifest source that served the archive.
    pub source: PackageSource,
    /// Every attempt the acquisition made for this release.
    pub attempts: Vec<SourceAttempt>,
}

/// The archives that one [`acquire_pending`] call obtained.
#[derive(Debug)]
#[non_exhaustive]
pub struct AcquiredArchives {
    /// Acquired archives keyed by their `bucket/game/mod` reference.
    pub archives: BTreeMap<String, AcquiredArchive>,
    /// Every attempt of every release, in acquisition order.
    pub attempts: Vec<SourceAttempt>,
}

/// The archive of every release that has one, keyed by reference.
pub(crate) fn archives_of(
    plan: &CatalogPlan,
    acquired: &AcquiredArchives,
) -> BTreeMap<String, PathBuf> {
    let mut archives = BTreeMap::new();
    for planned in &plan.order {
        if let Some(served) = &planned.served {
            archives.insert(planned.package.reference().to_string(), served.path.clone());
        }
    }
    for (key, entry) in &acquired.archives {
        archives.insert(key.clone(), entry.archive.clone());
    }
    archives
}

/// The digest of every release that has an archive, keyed by reference.
pub(crate) fn digests_of(
    plan: &CatalogPlan,
    acquired: &AcquiredArchives,
) -> BTreeMap<String, ReleaseDigest> {
    let mut digests = BTreeMap::new();
    for planned in &plan.order {
        if let Some(served) = &planned.served {
            digests.insert(
                planned.package.reference().to_string(),
                served.digest.clone(),
            );
        }
    }
    for (key, entry) in &acquired.archives {
        digests.insert(key.clone(), entry.digest.clone());
    }
    digests
}

/// The serving source of every release that has an archive, keyed by reference.
pub(crate) fn sources_of(
    plan: &CatalogPlan,
    acquired: &AcquiredArchives,
) -> BTreeMap<String, PackageSource> {
    let mut sources = BTreeMap::new();
    for planned in &plan.order {
        if let Some(served) = &planned.served {
            sources.insert(
                planned.package.reference().to_string(),
                served.source.clone(),
            );
        }
    }
    for (key, entry) in &acquired.archives {
        sources.insert(key.clone(), entry.source.clone());
    }
    sources
}

/// Acquire every release in [`CatalogPlan::pending`].
///
/// Each release is fetched through [`ArchiveCache::acquire_release`] with the
/// supplied transport, resolver, source policy and cache use. A cancellation
/// propagates as `CANCELLED` unchanged and keeps no partial cache entry. The
/// plan is not modified; the returned map fixes each acquired release's archive
/// path, digest and serving source.
///
/// # Errors
///
/// Returns the acquisition error of the first release that cannot be acquired,
/// including [`crate::error::ErrorCode::Cancelled`] when the control is
/// cancelled.
pub fn acquire_pending(
    plan: &CatalogPlan,
    cache: &ArchiveCache,
    cache_use: CacheUse,
    transport: &dyn SourceTransport,
    source_policy: SourcePolicy,
    resolver: &dyn ReleaseResolver,
    control: &OperationControl,
) -> Result<AcquiredArchives> {
    let mut archives = BTreeMap::new();
    let mut attempts = Vec::new();
    for pending in &plan.pending {
        let planned = plan
            .order
            .iter()
            .find(|planned| planned.package.reference() == pending.reference)
            .with_context(|| {
                format!(
                    "the plan does not name the pending release {}",
                    pending.reference
                )
            })?;
        let acquired = cache.acquire_release(
            &planned.package,
            cache_use,
            transport,
            source_policy,
            resolver,
            control,
        )?;
        if let (Some(expected), Some(found)) = (planned.digest.as_str(), acquired.digest.as_str()) {
            if expected != found {
                return Err(coded(
                    ErrorCode::DigestMismatch,
                    format!(
                        "{}: the acquired archive digest does not match the plan",
                        pending.reference
                    ),
                ));
            }
        }
        attempts.extend(acquired.attempts.iter().cloned());
        archives.insert(
            pending.reference.to_string(),
            AcquiredArchive {
                reference: pending.reference.clone(),
                archive: acquired.path,
                digest: acquired.digest,
                source: acquired.source,
                attempts: acquired.attempts,
            },
        );
    }
    Ok(AcquiredArchives { archives, attempts })
}
