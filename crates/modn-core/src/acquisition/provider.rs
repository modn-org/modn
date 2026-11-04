use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Read;

/// An exact GitHub release asset selector. Resolution grants no executable approval.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct GithubReleaseAsset {
    /// Repository in `owner/name` form.
    pub repository: String,
    /// Exact release tag. Moving `latest` selection is unsupported.
    pub tag: String,
    /// Exact case-sensitive asset filename.
    pub asset: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

fn segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

impl GithubReleaseAsset {
    /// Construct an exact selector. Resolution validates all fields before fetching.
    #[must_use]
    pub fn new(repository: String, tag: String, asset: String) -> Self {
        Self {
            repository,
            tag,
            asset,
        }
    }

    /// Validate the selector and return its unauthenticated GitHub API endpoint.
    pub fn metadata_url(&self) -> Result<String> {
        let parts: Vec<_> = self.repository.split('/').collect();
        if parts.len() != 2
            || !parts.iter().all(|part| segment(part))
            || !segment(&self.tag)
            || self.tag == "latest"
            || !segment(&self.asset)
        {
            bail!("invalid GitHub repository, pinned tag or asset name");
        }
        Ok(format!(
            "https://api.github.com/repos/{}/releases/tags/{}",
            self.repository, self.tag
        ))
    }

    /// Fetch bounded metadata and resolve exactly one asset to its canonical HTTPS URL.
    /// The caller supplies HTTP transport and must reject unsafe redirects.
    /// Pass the result to `ArchiveCache::acquire` with the manifest's pinned SHA-256.
    pub fn resolve<R: Read>(&self, fetch: impl FnOnce(&str) -> Result<R>) -> Result<String> {
        let url = self.metadata_url()?;
        let mut bytes = Vec::new();
        fetch(&url)?.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 1024 * 1024 {
            bail!("GitHub release metadata exceeds size limit");
        }
        let release: Release =
            serde_json::from_slice(&bytes).context("invalid GitHub release metadata")?;
        let release_url = format!(
            "https://github.com/{}/releases/tag/{}",
            self.repository, self.tag
        );
        if release.tag_name != self.tag || release.html_url != release_url {
            bail!("GitHub release repository or tag mismatch");
        }
        let mut matches = release
            .assets
            .iter()
            .filter(|asset| asset.name == self.asset);
        let asset = matches.next().context("GitHub release asset not found")?;
        if matches.next().is_some() {
            bail!("duplicate GitHub release asset");
        }
        let expected = format!(
            "https://github.com/{}/releases/download/{}/{}",
            self.repository, self.tag, self.asset
        );
        if asset.browser_download_url != expected {
            bail!("unsafe or mismatched GitHub asset URL");
        }
        Ok(expected)
    }
}

#[cfg(test)]
mod tests;
