//! Upstream queries of `modn catalog checkver`.

use super::checkver::Upstream;
use crate::nexus_auth;
use anyhow::{anyhow, bail, Context, Result};
use modn_core::buckets::{Checkver, PackageManifest, PackageSource};
use modn_core::checkver::{
    git_branch_commit, git_source_version, github_releases_url, nexus_files_url,
    select_github_release, select_nexus_file, test_endpoints_enabled, upstream_is_newer,
    NexusKeyOverride,
};
use std::env;
use std::io::Read;
use std::time::Duration;

const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) fn check_upstream(
    manifest: &PackageManifest,
    checkver: &Checkver,
) -> Result<(Option<String>, bool, Option<Upstream>)> {
    match checkver {
        Checkver::GithubRelease {
            repository,
            asset_pattern,
            prerelease,
            ..
        } => {
            require_source(manifest, "github-release", |source| {
                matches!(source, PackageSource::GithubRelease { .. })
            })?;
            let url = github_releases_url(repository)?;
            let body = fetch(&url, &github_headers(&url))?;
            let selected = select_github_release(&body, *prerelease, asset_pattern)?;
            let update = upstream_is_newer(&manifest.version, &selected.version);
            let upstream = Upstream::GithubRelease {
                repository: repository.clone(),
                tag: selected.tag,
                asset: selected.asset,
                version: selected.version.clone(),
            };
            Ok((Some(selected.version), update, Some(upstream)))
        }
        Checkver::Git {
            repository, branch, ..
        } => {
            let commit = manifest.sources.iter().find_map(|source| match source {
                PackageSource::Git { commit, .. } => Some(commit.as_str()),
                _ => None,
            });
            let commit = commit.context(
                "checkver kind git does not match any package source; a git check needs a git source",
            )?;
            let head = git_branch_commit(repository, branch)?;
            let latest = git_source_version(&head)?;
            let upstream = Upstream::Git {
                commit: head.clone(),
                version: latest.clone(),
            };
            Ok((Some(latest), head != commit, Some(upstream)))
        }
        Checkver::Nexus {
            game_domain,
            mod_id,
            file_pattern,
            ..
        } => {
            let file_id = manifest.sources.iter().find_map(|source| match source {
                PackageSource::Nexus { file_id, .. } => Some(*file_id),
                _ => None,
            });
            let file_id = file_id.context(
                "checkver kind nexus does not match any package source; a nexus check needs a nexus source",
            )?;
            let key = nexus_key()?;
            let url = nexus_files_url(game_domain, *mod_id);
            let body = fetch(
                &url,
                &[
                    ("apikey", key.expose_secret().to_owned()),
                    ("Application-Name", "Modn".to_owned()),
                    ("Application-Version", env!("CARGO_PKG_VERSION").to_owned()),
                    ("Accept", "application/json".to_owned()),
                ],
            )?;
            let selected = select_nexus_file(&body, file_pattern)?;
            Ok((None, selected.file_id != file_id, None))
        }
        _ => bail!("unsupported checkver kind"),
    }
}

fn require_source(
    manifest: &PackageManifest,
    kind: &str,
    accepts: impl Fn(&PackageSource) -> bool,
) -> Result<()> {
    if manifest.sources.iter().any(accepts) {
        return Ok(());
    }
    bail!(
        "checkver kind {kind} does not match any package source; add a {kind} source or remove the checkver block"
    )
}

fn nexus_key() -> Result<modn_core::acquisition::NexusApiKey> {
    match modn_core::checkver::nexus_key_override() {
        Some(NexusKeyOverride::Missing) => bail!(nexus_auth::MISSING_KEY),
        Some(NexusKeyOverride::Present(value)) => modn_core::acquisition::NexusApiKey::new(&value)
            .context("MODN_TEST_CHECKVER_NEXUS_KEY is not a valid Nexus API key"),
        Some(_) => bail!("unsupported Nexus API key override"),
        None => nexus_auth::load_key(),
    }
}

fn github_headers(url: &str) -> Vec<(&'static str, String)> {
    let mut headers = vec![("Accept", "application/vnd.github+json".to_owned())];
    if url.starts_with("https://api.github.com/") {
        if let Ok(token) = env::var("GITHUB_TOKEN") {
            let token = token.trim();
            if !token.is_empty() {
                headers.push(("Authorization", format!("Bearer {token}")));
            }
        }
    }
    headers
}

fn fetch(url: &str, headers: &[(&str, String)]) -> Result<Vec<u8>> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(!test_endpoints_enabled())
        .max_redirects(0)
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_global(Some(TOTAL_TIMEOUT))
        .user_agent("modn")
        .build()
        .into();
    let mut request = agent.get(url);
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let response = request.call().map_err(|error| match error {
        ureq::Error::StatusCode(status) => {
            anyhow!("request to {url} failed with HTTP status {status}")
        }
        other => anyhow!("request to {url} failed: {other}"),
    })?;
    let mut bytes = Vec::new();
    response
        .into_body()
        .into_reader()
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("cannot read the response from {url}"))?;
    if bytes.len() as u64 > MAX_RESPONSE_BYTES {
        bail!("response from {url} exceeds the 1 MiB size limit");
    }
    Ok(bytes)
}
