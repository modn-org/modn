//! `modn catalog checkver --update`: rewrite manifests with verified pins.

use super::checkver::{Marker, UpdateResult, Upstream};
use anyhow::{anyhow, bail, Context, Result};
use modn_core::acquisition::{GithubReleaseAsset, SourcePolicy, MAX_DOWNLOAD_BYTES};
use modn_core::buckets::{PackageManifest, PackageSource};
use modn_core::checkver::{
    archive_format_for_asset, github_asset_url, rewrite_manifest, test_endpoints_enabled,
    ManifestUpdate, SourceUpdate,
};
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;

pub(super) fn apply(
    game: &str,
    id: &str,
    path: &Path,
    manifest: &PackageManifest,
    upstream: &Upstream,
    keep_previous: bool,
) -> Result<UpdateResult> {
    let bytes = fs::read(path)
        .with_context(|| format!("cannot read package manifest: {}", path.display()))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| anyhow!("package manifest is not UTF-8: {}", path.display()))?;
    let index = source_index(manifest, upstream)?;
    let (new_version, new_text, marker, removed_upstream_sha256) = match upstream {
        Upstream::GithubRelease {
            repository,
            tag,
            asset,
            version,
        } => {
            let selector = GithubReleaseAsset::new(repository.clone(), tag.clone(), asset.clone());
            selector
                .metadata_url()
                .context("the upstream release tag or asset cannot be recorded in a manifest")?;
            let canonical = selector.download_url();
            let url = github_asset_url(repository, tag, asset)?;
            if !test_endpoints_enabled() && url != canonical {
                bail!("refusing a non-canonical GitHub download URL: {url}");
            }
            let format = archive_format_for_asset(asset)?;
            let blake3 = download_and_hash(&url)?;
            let removed = manifest.upstream_sha256.is_some();
            let mut update =
                ManifestUpdate::new(version, index, SourceUpdate::github_release(tag, asset))
                    .with_archive(format, &blake3);
            if removed {
                update = update.dropping_upstream_sha256();
            }
            let new_text =
                rewrite_manifest(text, &update).context("cannot rewrite the package manifest")?;
            (version.clone(), new_text, Marker::Blake3(blake3), removed)
        }
        Upstream::Git {
            commit, version, ..
        } => {
            let update = ManifestUpdate::new(version, index, SourceUpdate::git(commit));
            let new_text =
                rewrite_manifest(text, &update).context("cannot rewrite the package manifest")?;
            (
                version.clone(),
                new_text,
                Marker::Commit(commit.clone()),
                false,
            )
        }
    };
    let candidate: PackageManifest =
        serde_json::from_str(&new_text).context("the updated package manifest is invalid")?;
    candidate
        .validate(game, id)
        .context("the updated package manifest is invalid")?;
    let kept_previous = if keep_previous {
        let name = format!("{id}@{}.json", manifest.version);
        write_previous(&path.with_file_name(&name), &bytes)?;
        Some(format!("mods/{game}/{name}"))
    } else {
        None
    };
    write_atomic(path, new_text.as_bytes())?;
    let reread = fs::read(path)
        .with_context(|| format!("cannot re-read package manifest: {}", path.display()))?;
    if reread != new_text.as_bytes() {
        bail!(
            "the rewritten package manifest does not match on re-read: {}",
            path.display()
        );
    }
    let reread: PackageManifest =
        serde_json::from_slice(&reread).context("the rewritten package manifest is invalid")?;
    reread
        .validate(game, id)
        .context("the rewritten package manifest is invalid")?;
    Ok(UpdateResult {
        old_version: manifest.version.clone(),
        new_version,
        marker,
        kept_previous,
        removed_upstream_sha256,
        other_sources: manifest.sources.len().saturating_sub(1),
    })
}

fn source_index(manifest: &PackageManifest, upstream: &Upstream) -> Result<usize> {
    manifest
        .sources
        .iter()
        .position(|source| match (upstream, source) {
            (
                Upstream::GithubRelease { repository, .. },
                PackageSource::GithubRelease {
                    repository: source_repository,
                    ..
                },
            ) => source_repository == repository,
            (
                Upstream::Git { repository, .. },
                PackageSource::Git {
                    repository: source_repository,
                    ..
                },
            ) => source_repository == repository,
            _ => false,
        })
        .context("the checkver repository does not match any package source")
}

fn download_and_hash(url: &str) -> Result<String> {
    let policy = SourcePolicy::default();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(!test_endpoints_enabled())
        .timeout_connect(Some(policy.connect_timeout()))
        .timeout_per_read(Some(policy.idle_timeout()))
        .user_agent("modn")
        .build()
        .into();
    let response = agent.get(url).call().map_err(|error| match error {
        ureq::Error::StatusCode(status) => {
            anyhow!("download of {url} failed with HTTP status {status}")
        }
        other => anyhow!("download of {url} failed: {other}"),
    })?;
    let mut reader = response.into_body().into_reader();
    let mut file = tempfile::Builder::new()
        .prefix("modn-checkver-download-")
        .tempfile()
        .context("cannot create a temporary download file")?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        let read = reader
            .read(&mut buffer)
            .with_context(|| format!("cannot read the download from {url}"))?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > MAX_DOWNLOAD_BYTES {
            bail!("download from {url} exceeds the {MAX_DOWNLOAD_BYTES} byte size limit");
        }
        file.write_all(&buffer[..read])
            .context("cannot write the temporary download file")?;
        hasher.update(&buffer[..read]);
    }
    file.as_file_mut()
        .flush()
        .context("cannot flush the temporary download file")?;
    file.as_file()
        .sync_all()
        .context("cannot sync the temporary download file")?;
    Ok(hasher.finalize().to_hex().to_string())
}

fn write_previous(path: &Path, bytes: &[u8]) -> Result<()> {
    match fs::read(path) {
        Ok(existing) if existing == bytes => return Ok(()),
        Ok(_) => bail!(
            "previous release manifest already exists with different bytes: {}",
            path.display()
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("cannot read previous manifest: {}", path.display()))
        }
    }
    let directory = path
        .parent()
        .context("previous release manifest has no parent directory")?;
    let mut temp = tempfile::NamedTempFile::new_in(directory)
        .context("cannot create a temporary previous manifest")?;
    temp.write_all(bytes)
        .context("cannot write the previous manifest")?;
    temp.as_file_mut()
        .flush()
        .context("cannot flush the previous manifest")?;
    temp.as_file()
        .sync_all()
        .context("cannot sync the previous manifest")?;
    temp.persist_noclobber(path).map_err(|error| {
        if error.error.kind() == io::ErrorKind::AlreadyExists {
            anyhow!(
                "previous release manifest already exists: {}",
                path.display()
            )
        } else {
            anyhow!("cannot write {}: {}", path.display(), error.error)
        }
    })?;
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let directory = path
        .parent()
        .context("package manifest has no parent directory")?;
    let mut temp = tempfile::NamedTempFile::new_in(directory)
        .context("cannot create a temporary package manifest")?;
    temp.write_all(bytes)
        .context("cannot write the temporary package manifest")?;
    temp.as_file_mut()
        .flush()
        .context("cannot flush the temporary package manifest")?;
    temp.as_file()
        .sync_all()
        .context("cannot sync the temporary package manifest")?;
    temp.persist(path).map_err(|error| {
        anyhow!(
            "cannot replace package manifest {}: {}",
            path.display(),
            error.error
        )
    })?;
    Ok(())
}
