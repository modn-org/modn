//! Source, pin and dependency validation for package manifests.

use super::super::version::VersionReq;
use super::{nexus, valid_id, ArchiveFormat, PackageManifest, PackageSource};
use anyhow::{bail, Context, Result};
use std::collections::HashSet;

const MAX_EXECUTABLES: usize = 16;
const MAX_CONFLICTS: usize = 32;

pub(super) fn valid_tool_executable(path: &str) -> bool {
    path.len() <= 255
        && path.ends_with(".exe")
        && !path.starts_with('/')
        && !path.contains(['\\', ':'])
        && !path.chars().any(char::is_control)
        && path.split('/').any(|part| part == "tools")
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn valid_https(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or("");
    !host.is_empty()
        && !host.contains('@')
        && !host.contains('#')
        && !host.contains('?')
        && !url.chars().any(char::is_whitespace)
}

impl PackageManifest {
    /// Return the first artifact source, which validated records always have.
    ///
    /// # Panics
    ///
    /// Panics when the record was not validated and carries no source.
    #[must_use]
    pub fn primary_source(&self) -> &PackageSource {
        self.sources
            .first()
            .expect("validated package manifest has at least one source")
    }

    /// Return the BLAKE3 archive pin, when the release pins an archive.
    #[must_use]
    pub fn pin(&self) -> Option<&str> {
        self.blake3.as_deref()
    }

    fn valid_digest(value: &str) -> bool {
        value.len() == 64
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }

    pub(super) fn required_blake3(&self) -> Result<&str> {
        let pin = self
            .blake3
            .as_deref()
            .context("package blake3 is required for non-git sources")?;
        if !Self::valid_digest(pin) {
            bail!("package blake3 must be 64 lowercase hex characters");
        }
        Ok(pin)
    }

    pub(in crate::buckets) fn validate(&self, game: &str, id: &str) -> Result<()> {
        if self.schema_version != 2 {
            bail!(
                "bucket package schema_version {} is unsupported; Modn requires bucket contract v2 and does not read v1 records",
                self.schema_version
            );
        }
        if !valid_id(&self.id) || !valid_id(&self.game) || self.id != id || self.game != game {
            bail!("bucket package identity disagrees with mods/<game>/<id>.json path");
        }
        if self.sources.is_empty() {
            bail!("bucket package sources must not be empty");
        }
        if let Some(upstream) = &self.upstream_sha256 {
            if !Self::valid_digest(upstream) {
                bail!("package upstream_sha256 must be 64 lowercase hex characters");
            }
        }
        if self.name.trim().is_empty() || self.name.len() > 128 {
            bail!("invalid package name");
        }
        if self.version.is_empty()
            || self.version.len() > 64
            || !self
                .version
                .bytes()
                .all(|b| b.is_ascii_graphic() && b != b'/' && b != b'\\')
        {
            bail!("invalid package version");
        }
        let git_only = self
            .sources
            .iter()
            .all(|source| matches!(source, PackageSource::Git { .. }));
        if git_only {
            if self.blake3.is_some() {
                bail!("git source packages must not declare blake3; the commit is the pin");
            }
        } else {
            self.required_blake3()?;
        }
        for source in &self.sources {
            match source {
                PackageSource::Git { repository, commit } => {
                    if self
                        .archive_format
                        .as_ref()
                        .is_some_and(|format| !matches!(format, ArchiveFormat::Zip))
                    {
                        bail!("git source archive_format must be zip when present");
                    }
                    crate::acquisition::GitSource::new(repository.clone(), commit.clone())
                        .remote_url()?;
                }
                PackageSource::GithubRelease {
                    repository,
                    tag,
                    asset,
                } => {
                    crate::acquisition::GithubReleaseAsset::new(
                        repository.clone(),
                        tag.clone(),
                        asset.clone(),
                    )
                    .metadata_url()?;
                }
                PackageSource::Manual { filename } => {
                    let format = self
                        .archive_format
                        .as_ref()
                        .context("archive_format is required for non-git sources")?;
                    let extension = match format {
                        ArchiveFormat::Zip => ".zip",
                        ArchiveFormat::SevenZ => ".7z",
                    };
                    if filename.is_empty()
                        || filename.len() > 255
                        || filename == "."
                        || filename == ".."
                        || filename
                            .chars()
                            .any(|c| c.is_control() || c == '/' || c == '\\')
                        || !filename.to_ascii_lowercase().ends_with(extension)
                    {
                        bail!("invalid manual archive filename");
                    }
                }
                PackageSource::Nexus {
                    game_domain,
                    mod_id,
                    file_id,
                    file_name,
                } => nexus::validate(self, game_domain, *mod_id, *file_id, file_name)?,
                PackageSource::Https { url } => {
                    self.archive_format
                        .as_ref()
                        .context("archive_format is required for non-git sources")?;
                    if !valid_https(url) {
                        bail!("download URL must be HTTPS")
                    }
                }
            }
        }
        if self.homepage.as_ref().is_some_and(|url| !valid_https(url)) {
            bail!("homepage URL must be HTTPS");
        }
        let mut declared = HashSet::new();
        if self.executables.len() > MAX_EXECUTABLES
            || !self
                .executables
                .iter()
                .all(|path| valid_tool_executable(path) && declared.insert(path))
        {
            bail!("invalid or duplicate declared tool executable");
        }
        let mut unique = HashSet::new();
        for dependency in &self.dependencies {
            if !valid_id(dependency) || dependency == &self.id || !unique.insert(dependency) {
                bail!("invalid, self-referential or duplicate dependency");
            }
        }
        for (dependency, requirement) in &self.dependency_versions {
            if !unique.contains(dependency) {
                bail!("dependency_versions names {dependency}, which is not a dependency");
            }
            if requirement.len() > 64 {
                bail!("version requirement for {dependency} is too long");
            }
            VersionReq::parse(requirement)
                .with_context(|| format!("invalid version requirement for {dependency}"))?;
        }
        let mut conflicts = HashSet::new();
        if self.conflicts.len() > MAX_CONFLICTS {
            bail!("too many conflicts");
        }
        for conflict in &self.conflicts {
            if !valid_id(conflict)
                || conflict == &self.id
                || unique.contains(conflict)
                || !conflicts.insert(conflict)
            {
                bail!("invalid, self-referential, duplicate or depended-on conflict: {conflict}");
            }
        }
        Ok(())
    }
}
