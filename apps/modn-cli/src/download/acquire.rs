use super::nexus_download;
use super::nexus_label;
use super::HttpTransport;
use crate::catalog_install::reference_of;
use crate::git_source;
use crate::nexus_auth;
use crate::nxm::NxmLink;
use crate::progress::Progress;
use modn_core::acquisition::{
    ArchiveCache, CacheUse, GitAcquired, GitCacheKey, NexusDownloadUrl, NexusFile, PlannedAction,
    ReleaseResolver, SourceAttempt, SourceBody, SourceCandidate, SourcePolicy, SourceTarget,
};
use modn_core::buckets::{BucketPackage, PackageSource};
use modn_core::progress::OperationControl;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

#[expect(
    clippy::too_many_arguments,
    reason = "the step keeps the cache, package set, report sink and cancellation explicit"
)]
pub(crate) fn download(
    cache: &ArchiveCache,
    cache_use: CacheUse,
    releases: &[&BucketPackage],
    archives: &mut BTreeMap<String, PathBuf>,
    digests: &mut BTreeMap<String, String>,
    served: &mut BTreeMap<String, PackageSource>,
    links: &[NxmLink],
    json: bool,
    progress: &Progress,
) -> anyhow::Result<Vec<SourceAttempt>> {
    let text = !json && !progress.enabled();
    let mut report = std::io::stderr();
    download_into(
        cache,
        cache_use,
        releases,
        archives,
        digests,
        served,
        links,
        text,
        &mut report,
        progress.control(),
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "the download step keeps the verified cache, package set, report sink and cancellation explicit"
)]
pub(super) fn download_into(
    cache: &ArchiveCache,
    cache_use: CacheUse,
    releases: &[&BucketPackage],
    archives: &mut BTreeMap<String, PathBuf>,
    digests: &mut BTreeMap<String, String>,
    served: &mut BTreeMap<String, PackageSource>,
    links: &[NxmLink],
    text: bool,
    report: &mut dyn Write,
    control: &OperationControl,
) -> anyhow::Result<Vec<SourceAttempt>> {
    let mut attempts = Vec::new();
    for release in releases {
        let reference = reference_of(release).to_string();
        if archives.contains_key(&reference) {
            continue;
        }
        let resolver = CliResolver {
            links,
            text,
            report: RefCell::new(&mut *report),
        };
        let acquired = cache.acquire_release(
            release,
            cache_use,
            &HttpTransport,
            SourcePolicy::default(),
            &resolver,
            control,
        )?;
        if let Some(digest) = acquired.digest.as_str() {
            digests.insert(reference.clone(), digest.to_owned());
        }
        archives.insert(reference.clone(), acquired.path);
        served.insert(reference, acquired.source);
        attempts.extend(acquired.attempts);
    }
    Ok(attempts)
}

/// The CLI steps that release acquisition keeps outside core.
struct CliResolver<'a> {
    links: &'a [NxmLink],
    text: bool,
    report: RefCell<&'a mut dyn Write>,
}

impl CliResolver<'_> {
    fn note(&self, line: String) {
        if self.text {
            let mut report = self.report.borrow_mut();
            let _ = writeln!(report, "{line}");
        }
    }
}

impl ReleaseResolver for CliResolver<'_> {
    fn git(
        &self,
        release: &BucketPackage,
        cache: &ArchiveCache,
        source: &PackageSource,
        key: &GitCacheKey<'_>,
        cache_use: CacheUse,
        control: &OperationControl,
    ) -> anyhow::Result<GitAcquired> {
        let PackageSource::Git { repository, .. } = source else {
            anyhow::bail!("the Git step received a non-Git source");
        };
        self.note(format!(
            "Fetching {} {} commit {} from {repository}",
            release.reference(),
            release.package.version,
            key.commit
        ));
        git_source::acquire(cache, release, repository, key, cache_use, control)
    }

    fn nexus(
        &self,
        release: &BucketPackage,
        source: &PackageSource,
    ) -> anyhow::Result<NexusDownloadUrl> {
        let PackageSource::Nexus {
            game_domain,
            mod_id,
            file_id,
            file_name,
        } = source
        else {
            anyhow::bail!("the Nexus step received a non-Nexus source");
        };
        let label = nexus_label(game_domain, *mod_id, *file_id, file_name);
        self.note(format!(
            "Downloading {} {} from {label}",
            release.reference(),
            release.package.version
        ));
        let file = NexusFile::new(game_domain, *mod_id, *file_id, file_name)?;
        let link = nexus_download::matching_link(self.links, game_domain, *mod_id, *file_id);
        let api_key = nexus_auth::load_key()?;
        nexus_download::resolve(&file, &api_key, link)
    }

    fn open_nexus(&self, link: &NexusDownloadUrl) -> anyhow::Result<SourceBody> {
        let (reader, total) = nexus_download::open(link)?;
        Ok(SourceBody::new(reader, total))
    }

    fn candidate(&self, release: &BucketPackage, action: PlannedAction) -> SourceCandidate {
        let candidate = super::fallback::resolve_candidate(action);
        if matches!(candidate.target, SourceTarget::Fetch(_)) {
            self.note(format!(
                "Downloading {} {} from {}",
                release.reference(),
                release.package.version,
                candidate.label
            ));
        }
        candidate
    }

    fn attempt(&self, release: &BucketPackage, attempt: &SourceAttempt) {
        self.note(format!(
            "Source {}: {}: {}",
            release.reference(),
            attempt.source,
            attempt.describe()
        ));
    }
}
