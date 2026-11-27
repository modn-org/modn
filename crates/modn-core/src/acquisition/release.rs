//! Release acquisition from a package's planned sources.
//!
//! A release names its archive sources in manifest order. Core chooses that
//! order, the cache key of every source and the serving source, so every front
//! end agrees. The caller supplies only the steps that need provider
//! credentials, a Git process or provider metadata.

use super::{
    plan_sources, source_cache_url, ArchiveCache, ArchiveDigest, CacheKey, CacheUse, FetchError,
    GitAcquired, GitCacheKey, NexusDownloadUrl, PlannedAction, ReleaseDigest, SourceAttempt,
    SourceBody, SourceCandidate, SourceFailure, SourcePolicy, SourceTarget, SourceTransport,
    SourcesError,
};
use crate::buckets::{ArchiveFormat, BucketPackage, PackageManifest, PackageSource};
use crate::error::{code_of, ErrorCode};
use crate::progress::OperationControl;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A verified archive for one release and every attempt the acquisition made.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ReleaseAcquired {
    /// The verified cache entry that serves the release.
    pub path: PathBuf,
    /// The digest of the archive, and whether the catalog pinned it.
    pub digest: ReleaseDigest,
    /// The manifest source that supplied the archive.
    pub source: PackageSource,
    /// Every attempt, in the order the acquisition made them.
    pub attempts: Vec<SourceAttempt>,
}

/// The provider steps that release acquisition keeps in its caller.
///
/// Nexus credentials and `nxm` links, the Git process and GitHub metadata
/// resolution never enter core. Each method returns data that core checks,
/// records and orders with the other sources.
pub trait ReleaseResolver {
    /// Acquire the planned Git source, reporting progress and honouring
    /// `control`.
    fn git(
        &self,
        release: &BucketPackage,
        cache: &ArchiveCache,
        source: &PackageSource,
        key: &GitCacheKey<'_>,
        cache_use: CacheUse,
        control: &OperationControl,
    ) -> Result<GitAcquired>;

    /// Resolve the planned Nexus file to a CDN link, or refuse it.
    fn nexus(&self, release: &BucketPackage, source: &PackageSource) -> Result<NexusDownloadUrl>;

    /// Open the CDN link that [`ReleaseResolver::nexus`] resolved.
    fn open_nexus(&self, link: &NexusDownloadUrl) -> Result<SourceBody>;

    /// Resolve a planned fetch or GitHub source into one candidate.
    fn candidate(&self, release: &BucketPackage, action: PlannedAction) -> SourceCandidate;

    /// Observe one attempt after core records it.
    ///
    /// Core calls this in the order it makes the attempts, before the next
    /// source starts. A front end prints its text note here, so the note keeps
    /// the attempt order. The default does nothing.
    fn attempt(&self, _release: &BucketPackage, _attempt: &SourceAttempt) {}
}

impl ArchiveCache {
    /// Return the verified cache entry that serves `release`, if one exists.
    ///
    /// The lookup reads the cache only. It calls no resolver or transport. With
    /// several verified entries, the source that owns the first entry in
    /// manifest order is named. A Git entry reports its computed digest; an
    /// archive entry reports the catalog pin.
    ///
    /// # Errors
    ///
    /// Returns the error of an unreadable cache entry or an invalid source.
    pub fn cached_release(&self, release: &BucketPackage) -> Result<Option<ReleaseAcquired>> {
        for source in &release.package.sources {
            match source {
                PackageSource::Git { commit, .. } => {
                    let key = git_key(release, commit);
                    if let Some((path, blake3)) = self.git_cached(&key)? {
                        return Ok(Some(ReleaseAcquired {
                            path,
                            digest: ReleaseDigest::computed(&blake3)?,
                            source: source.clone(),
                            attempts: Vec::new(),
                        }));
                    }
                }
                _ => {
                    let Some(url) = source_cache_url(source)? else {
                        continue;
                    };
                    let key = release_key(release, &url)?;
                    let expected = self.path(&key);
                    if !expected.is_file() {
                        continue;
                    }
                    if self.cached(&key)? == Some(expected.clone()) {
                        return Ok(Some(ReleaseAcquired {
                            path: expected,
                            digest: ReleaseDigest::from_pin(release.package.pin())?,
                            source: source.clone(),
                            attempts: Vec::new(),
                        }));
                    }
                }
            }
        }
        for source in &release.package.sources {
            let Some(url) = source_cache_url(source)? else {
                continue;
            };
            let probe = release_key(release, &url)?;
            let Some(path) = self.cached(&probe)? else {
                continue;
            };
            let owner = release
                .package
                .sources
                .iter()
                .find(|candidate| {
                    source_cache_url(candidate)
                        .ok()
                        .flatten()
                        .is_some_and(|url| {
                            release_key(release, &url).is_ok_and(|key| self.path(&key) == path)
                        })
                })
                .cloned()
                .unwrap_or_else(|| source.clone());
            return Ok(Some(ReleaseAcquired {
                path,
                digest: ReleaseDigest::from_pin(release.package.pin())?,
                source: owner,
                attempts: Vec::new(),
            }));
        }
        Ok(None)
    }

    /// Return a verified archive for `release`, trying its sources in order.
    ///
    /// With [`CacheUse::Reuse`], a verified cache entry for any source of the
    /// release is returned before any resolver or transport call. Otherwise
    /// every planned source is tried in manifest order through `resolver` and
    /// `transport`. A digest mismatch is final for that source. A failed
    /// attempt leaves no partial file or cache entry behind.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::Cancelled`] when `control` is cancelled, the error
    /// of a lone failed Git or Nexus source, and
    /// [`ErrorCode::DownloadFailed`] with a [`SourcesError`] when every source
    /// fails.
    pub fn acquire_release(
        &self,
        release: &BucketPackage,
        cache_use: CacheUse,
        transport: &dyn SourceTransport,
        policy: SourcePolicy,
        resolver: &dyn ReleaseResolver,
        control: &OperationControl,
    ) -> Result<ReleaseAcquired> {
        control.checkpoint()?;
        let reference = release.reference().to_string();
        if cache_use == CacheUse::Reuse {
            if let Some(cached) = self.cached_release(release)? {
                return Ok(cached);
            }
        }
        let mut attempts = Vec::new();
        let mut specific: Option<anyhow::Error> = None;
        for planned in plan_sources(release) {
            control.checkpoint()?;
            let source = &planned.source;
            match planned.action {
                PlannedAction::Git { repository, commit } => {
                    let label = format!("git {repository} commit {commit}");
                    let key = git_key(release, &commit);
                    match resolver.git(release, self, source, &key, cache_use, control) {
                        Ok(acquired) => {
                            observed(
                                resolver,
                                release,
                                &mut attempts,
                                SourceAttempt::ok(label, 1),
                            );
                            return Ok(ReleaseAcquired {
                                path: acquired.path,
                                digest: ReleaseDigest::computed(&acquired.blake3)?,
                                source: source.clone(),
                                attempts,
                            });
                        }
                        Err(error) if ends_acquisition(&error) => return Err(error),
                        Err(error) => {
                            observed(
                                resolver,
                                release,
                                &mut attempts,
                                SourceAttempt::failed(label, 1, classify(&error)),
                            );
                            specific = Some(error);
                        }
                    }
                }
                PlannedAction::Nexus {
                    game_domain,
                    mod_id,
                    file_id,
                    file_name,
                } => {
                    let label = nexus_label(&game_domain, mod_id, file_id, &file_name);
                    match resolver.nexus(release, source) {
                        Err(error) => {
                            if ends_acquisition(&error) {
                                return Err(error);
                            }
                            observed(
                                resolver,
                                release,
                                &mut attempts,
                                SourceAttempt::failed(label, 1, classify(&error)),
                            );
                            specific = Some(error);
                        }
                        Ok(link) => {
                            let Some(url) = source_cache_url(source)? else {
                                return Err(anyhow::anyhow!(
                                    "a Nexus source has no archive cache URL"
                                ));
                            };
                            let key = release_key(release, &url)?;
                            let outcome = self.acquire_with_control(
                                &key,
                                cache_use,
                                Some(reference.clone()),
                                control,
                                |_| {
                                    let body = resolver.open_nexus(&link)?;
                                    Ok((body.reader, body.total))
                                },
                            );
                            match outcome {
                                Ok(acquired) => {
                                    observed(
                                        resolver,
                                        release,
                                        &mut attempts,
                                        SourceAttempt::ok(label, 1),
                                    );
                                    return Ok(ReleaseAcquired {
                                        path: acquired.path,
                                        digest: ReleaseDigest::from_pin(release.package.pin())?,
                                        source: source.clone(),
                                        attempts,
                                    });
                                }
                                Err(error) if ends_acquisition(&error) => return Err(error),
                                Err(error) => {
                                    observed(
                                        resolver,
                                        release,
                                        &mut attempts,
                                        SourceAttempt::failed(label, 1, classify(&error)),
                                    );
                                    specific = Some(error);
                                }
                            }
                        }
                    }
                }
                action => {
                    let candidate = resolver.candidate(release, action);
                    let key = release_key(release, candidate_url(&candidate))?;
                    let outcome = self.acquire_from_sources(
                        &key,
                        cache_use,
                        std::slice::from_ref(&candidate),
                        transport,
                        policy,
                        Some(reference.clone()),
                        control,
                    );
                    match outcome {
                        Ok(acquired) => {
                            for attempt in acquired.attempts {
                                observed(resolver, release, &mut attempts, attempt);
                            }
                            return Ok(ReleaseAcquired {
                                path: acquired.acquired.path,
                                digest: ReleaseDigest::from_pin(release.package.pin())?,
                                source: source.clone(),
                                attempts,
                            });
                        }
                        Err(error) if ends_acquisition(&error) => return Err(error),
                        Err(error) => match attempts_of(&error) {
                            Some(failed) => {
                                for attempt in failed {
                                    observed(resolver, release, &mut attempts, attempt);
                                }
                            }
                            None => {
                                return Err(
                                    error.context(format!("source {reference} was not tried"))
                                )
                            }
                        },
                    }
                }
            }
        }
        match specific {
            Some(error) if attempts.len() == 1 => Err(error),
            _ => Err(SourcesError::new(reference, attempts).into()),
        }
    }
}

/// Build the one-source release record that a history entry describes.
///
/// Undo knows the release's identity, serving source and recorded archive
/// digest, but not the rest of its manifest. This record lets
/// [`ArchiveCache::acquire_release`] apply the same cache lookup and source
/// plan as an install. A Git record carries no archive pin, because its commit
/// is the integrity pin.
///
/// # Errors
///
/// Returns an error when a non-Git digest is not a BLAKE3 digest.
pub fn recorded_release(
    bucket: &str,
    game: &str,
    id: &str,
    version: &str,
    source: PackageSource,
    digest: &str,
) -> Result<BucketPackage> {
    let git = matches!(source, PackageSource::Git { .. });
    let blake3 = if git {
        None
    } else {
        Some(ArchiveDigest::new(digest)?.as_str().to_owned())
    };
    Ok(BucketPackage {
        bucket: bucket.to_owned(),
        package: PackageManifest {
            schema_version: 2,
            id: id.to_owned(),
            game: game.to_owned(),
            name: id.to_owned(),
            version: version.to_owned(),
            archive_format: if git { None } else { Some(ArchiveFormat::Zip) },
            blake3,
            sources: vec![source],
            redistributable: false,
            upstream_sha256: None,
            dependencies: Vec::new(),
            homepage: None,
            executables: Vec::new(),
            dependency_versions: BTreeMap::new(),
            conflicts: Vec::new(),
            native_injector: false,
            checkver: None,
        },
    })
}

fn observed(
    resolver: &dyn ReleaseResolver,
    release: &BucketPackage,
    attempts: &mut Vec<SourceAttempt>,
    attempt: SourceAttempt,
) {
    resolver.attempt(release, &attempt);
    attempts.push(attempt);
}

fn release_key<'a>(release: &'a BucketPackage, url: &'a str) -> Result<CacheKey<'a>> {
    let pin = release
        .package
        .pin()
        .context("an archive source needs a package BLAKE3 pin")?;
    let digest = ArchiveDigest::new(pin)?;
    Ok(CacheKey::new(
        &release.package.game,
        &release.package.id,
        &release.package.version,
        url,
        digest.as_str(),
    ))
}

fn git_key<'a>(release: &'a BucketPackage, commit: &'a str) -> GitCacheKey<'a> {
    GitCacheKey::new(
        &release.package.game,
        &release.package.id,
        &release.package.version,
        commit,
    )
}

fn nexus_label(game_domain: &str, mod_id: u64, file_id: u64, file_name: &str) -> String {
    format!("Nexus Mods: {game_domain} mod {mod_id} file {file_id} ({file_name})")
}

fn candidate_url(candidate: &SourceCandidate) -> &str {
    match &candidate.target {
        SourceTarget::Fetch(url) => url,
        _ => &candidate.label,
    }
}

/// Whether `error` ends the acquisition instead of trying the next source.
///
/// A cancelled operation and a Git process tree that Modn could not confirm
/// stopped both end the loop. A descendant of the tree may still write to the
/// staging directory, so no other source may start.
fn ends_acquisition(error: &anyhow::Error) -> bool {
    matches!(
        code_of(error),
        Some(ErrorCode::Cancelled | ErrorCode::GitProcessTreeNotStopped)
    )
}

fn classify(error: &anyhow::Error) -> SourceFailure {
    if let Some(fetch) = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<FetchError>())
    {
        return fetch.clone().into();
    }
    SourceFailure::Refused(format!("{error:#}"))
}

fn attempts_of(error: &anyhow::Error) -> Option<Vec<SourceAttempt>> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<SourcesError>())
        .map(|sources| sources.attempts().to_vec())
}

#[cfg(test)]
mod tests;
