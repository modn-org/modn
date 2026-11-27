//! `modn catalog checkver`: stateless upstream checks for catalog packages.

use super::keys::require_directory;
use crate::nexus_auth;
use anyhow::{anyhow, bail, Context, Result};
use modn_core::buckets::{Checkver, PackageManifest, PackageSource};
use modn_core::checkver::{
    git_branch_commit, git_source_version, github_releases_url, nexus_files_url,
    select_github_release, select_nexus_file, test_endpoints_enabled, upstream_is_newer,
    NexusKeyOverride,
};
use modn_core::error::{coded, ErrorCode};
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    UpToDate,
    UpdateAvailable,
    Skipped,
    Error,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Self::UpToDate => "up_to_date",
            Self::UpdateAvailable => "update_available",
            Self::Skipped => "skipped",
            Self::Error => "error",
        }
    }
}

struct Package {
    game: String,
    id: String,
    manifest: Option<PackageManifest>,
    load_error: Option<String>,
}

struct Report {
    game: String,
    id: String,
    status: Status,
    kind: Option<&'static str>,
    current_version: Option<String>,
    latest_version: Option<String>,
    error: Option<String>,
}

#[derive(Default)]
struct Summary {
    up_to_date: usize,
    update_available: usize,
    skipped: usize,
    error: usize,
}

pub(super) fn run(dir: &Path, target: Option<&str>, json_output: bool) -> Result<()> {
    require_directory(dir, "catalog directory")?;
    let packages = select(discover(dir)?, target)?;
    let reports: Vec<Report> = packages.iter().map(check).collect();
    let summary = summarize(&reports);
    if json_output {
        print_json(&reports, &summary);
    } else {
        print_human(&reports, &summary);
    }
    if summary.error > 0 {
        return Err(coded(
            ErrorCode::BucketFailed,
            format!("{} package check(s) failed", summary.error),
        ));
    }
    Ok(())
}

fn discover(dir: &Path) -> Result<Vec<Package>> {
    let mods = dir.join("mods");
    if fs::symlink_metadata(&mods).is_err() {
        return Ok(Vec::new());
    }
    let mut packages = Vec::new();
    let mut games: Vec<_> = fs::read_dir(&mods)
        .with_context(|| format!("cannot read catalog packages: {}", mods.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    games.sort_by_key(std::fs::DirEntry::file_name);
    for game in games {
        if !game.file_type()?.is_dir() {
            continue;
        }
        let game_id = game.file_name().to_string_lossy().into_owned();
        let mut records: Vec<_> = fs::read_dir(game.path())
            .with_context(|| {
                format!(
                    "cannot read catalog game directory: {}",
                    game.path().display()
                )
            })?
            .collect::<std::io::Result<Vec<_>>>()?;
        records.sort_by_key(std::fs::DirEntry::file_name);
        for entry in records {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(std::ffi::OsStr::to_str) else {
                continue;
            };
            let Some(id) = name.strip_suffix(".json") else {
                continue;
            };
            if id.is_empty() || id.contains('@') || !entry.file_type()?.is_file() {
                continue;
            }
            packages.push(read_package(&game_id, id, &path));
        }
    }
    packages.sort_by(|left, right| (&left.game, &left.id).cmp(&(&right.game, &right.id)));
    Ok(packages)
}

fn read_package(game: &str, id: &str, path: &Path) -> Package {
    let loaded =
        read_manifest(path).and_then(|manifest| manifest.validate(game, id).map(|()| manifest));
    match loaded {
        Ok(manifest) => Package {
            game: game.to_owned(),
            id: id.to_owned(),
            manifest: Some(manifest),
            load_error: None,
        },
        Err(error) => Package {
            game: game.to_owned(),
            id: id.to_owned(),
            manifest: None,
            load_error: Some(format!("{error:#}")),
        },
    }
}

fn read_manifest(path: &Path) -> Result<PackageManifest> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("cannot read package manifest: {}", path.display()))?;
    if !metadata.is_file() {
        bail!(
            "package manifest must be a regular file: {}",
            path.display()
        );
    }
    if metadata.len() > MAX_MANIFEST_BYTES {
        bail!("package manifest exceeds 1 MiB: {}", path.display());
    }
    let bytes = fs::read(path)
        .with_context(|| format!("cannot read package manifest: {}", path.display()))?;
    serde_json::from_slice(&bytes).context("invalid package manifest")
}

fn select(packages: Vec<Package>, target: Option<&str>) -> Result<Vec<Package>> {
    let Some(target) = target else {
        return Ok(packages);
    };
    let (game, id) = match target.split_once('/') {
        Some((game, id)) if !game.is_empty() && !id.is_empty() && !id.contains('/') => {
            (Some(game), id)
        }
        Some(_) => bail!("invalid package reference; expected <mod-id> or <game-id>/<mod-id>"),
        None => (None, target),
    };
    if id.is_empty() {
        bail!("invalid package reference; expected <mod-id> or <game-id>/<mod-id>");
    }
    let matches: Vec<Package> = packages
        .into_iter()
        .filter(|package| package.id == id && game.is_none_or(|game| package.game == game))
        .collect();
    match matches.len() {
        0 => bail!("catalog package not found: {target}"),
        1 => Ok(matches),
        _ => {
            let games: Vec<&str> = matches
                .iter()
                .map(|package| package.game.as_str())
                .collect();
            bail!(
                "catalog package {target} is ambiguous; it exists in games {}; use <game-id>/<mod-id>",
                games.join(", ")
            )
        }
    }
}

fn check(package: &Package) -> Report {
    let Some(manifest) = &package.manifest else {
        return Report {
            game: package.game.clone(),
            id: package.id.clone(),
            status: Status::Error,
            kind: None,
            current_version: None,
            latest_version: None,
            error: package.load_error.clone(),
        };
    };
    let Some(checkver) = &manifest.checkver else {
        return Report {
            game: package.game.clone(),
            id: package.id.clone(),
            status: Status::Skipped,
            kind: None,
            current_version: Some(manifest.version.clone()),
            latest_version: None,
            error: None,
        };
    };
    let kind = Some(kind_of(checkver));
    match check_upstream(manifest, checkver) {
        Ok((latest, update)) => Report {
            game: package.game.clone(),
            id: package.id.clone(),
            status: if update {
                Status::UpdateAvailable
            } else {
                Status::UpToDate
            },
            kind,
            current_version: Some(manifest.version.clone()),
            latest_version: latest,
            error: None,
        },
        Err(error) => Report {
            game: package.game.clone(),
            id: package.id.clone(),
            status: Status::Error,
            kind,
            current_version: Some(manifest.version.clone()),
            latest_version: None,
            error: Some(format!("{error:#}")),
        },
    }
}

fn kind_of(checkver: &Checkver) -> &'static str {
    match checkver {
        Checkver::GithubRelease { .. } => "github-release",
        Checkver::Git { .. } => "git",
        Checkver::Nexus { .. } => "nexus",
        _ => "unknown",
    }
}

fn check_upstream(
    manifest: &PackageManifest,
    checkver: &Checkver,
) -> Result<(Option<String>, bool)> {
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
            Ok((Some(selected.version), update))
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
            Ok((Some(latest), head != commit))
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
            Ok((None, selected.file_id != file_id))
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

fn summarize(reports: &[Report]) -> Summary {
    let mut summary = Summary::default();
    for report in reports {
        match report.status {
            Status::UpToDate => summary.up_to_date += 1,
            Status::UpdateAvailable => summary.update_available += 1,
            Status::Skipped => summary.skipped += 1,
            Status::Error => summary.error += 1,
        }
    }
    summary
}

fn print_json(reports: &[Report], summary: &Summary) {
    let packages: Vec<Value> = reports
        .iter()
        .map(|report| {
            json!({
                "game": report.game,
                "id": report.id,
                "status": report.status.as_str(),
                "kind": report.kind,
                "current_version": report.current_version,
                "latest_version": report.latest_version,
                "error": report.error,
            })
        })
        .collect();
    let value = json!({
        "packages": packages,
        "summary": {
            "up_to_date": summary.up_to_date,
            "update_available": summary.update_available,
            "skipped": summary.skipped,
            "error": summary.error,
        },
    });
    println!("{value}");
}

fn print_human(reports: &[Report], summary: &Summary) {
    for report in reports {
        let name = format!("{}/{}", report.game, report.id);
        match report.status {
            Status::Error => println!(
                "{name}: error: {}",
                report.error.as_deref().unwrap_or("unknown error")
            ),
            Status::Skipped => println!(
                "{name}: skipped {}",
                report.current_version.as_deref().unwrap_or("?")
            ),
            status => println!(
                "{name}: {} {} -> {}",
                status.as_str(),
                report.current_version.as_deref().unwrap_or("?"),
                report.latest_version.as_deref().unwrap_or("unknown")
            ),
        }
    }
    println!(
        "Summary: {} up_to_date, {} update_available, {} skipped, {} error",
        summary.up_to_date, summary.update_available, summary.skipped, summary.error
    );
}
