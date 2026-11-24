//! Select the newest `modn-org/main` release that this Modn build supports.
//!
//! Each release may carry a small asset, `main-bucket.requires.json`, with the
//! bucket contract `schema_version` and the lowest Modn version that reads the
//! release. GitHub returns the release list newest first. The selector takes
//! the first release that this build reads. A release without the asset is
//! treated as readable, because releases from before this rule carry none; the
//! install checks still refuse a bucket this build does not support.

use anyhow::{bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::io::Read;

/// The release list that GitHub returns newest first.
pub(super) const DEFAULT_RELEASES: &str = "https://api.github.com/repos/modn-org/main/releases";
const RELEASE_LIST_MAX_BYTES: u64 = 1024 * 1024;
const REQUIRES_MAX_BYTES: u64 = 8 * 1024;
const REQUIRES_ASSET: &str = "main-bucket.requires.json";
const ZIP_ASSET: &str = "main-bucket.zip";
const SIGNATURE_ASSET: &str = "main-bucket.zip.minisig";
/// The bucket contract that this build reads.
const SUPPORTED_SCHEMA_VERSION: u32 = 2;

#[derive(Deserialize)]
struct Release {
    #[serde(default)]
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

#[derive(Deserialize)]
struct Requires {
    schema_version: u32,
    min_modn_version: String,
}

struct Requirement {
    schema_version: u32,
    minimum: semver::Version,
    minimum_text: String,
}

/// A newer release that this build cannot read.
pub(super) struct UnsupportedRelease {
    pub(super) tag: String,
    pub(super) min_modn_version: String,
    schema_version: u32,
}

impl UnsupportedRelease {
    pub(super) fn needs(&self) -> String {
        if self.schema_version == SUPPORTED_SCHEMA_VERSION {
            format!("needs Modn {}", self.min_modn_version)
        } else {
            format!(
                "needs Modn {} and bucket contract v{}",
                self.min_modn_version, self.schema_version
            )
        }
    }

    /// The text-mode notice that names the release Modn keeps.
    pub(super) fn notice(&self, selected: &str) -> String {
        format!(
            "main release {} {}; keeping {selected}. Update Modn to get it.",
            self.tag,
            self.needs()
        )
    }
}

/// The release that this build installs.
pub(super) struct Selection {
    pub(super) tag: String,
    pub(super) zip: String,
    pub(super) signature: String,
    pub(super) newer: Option<UnsupportedRelease>,
}

fn read_limited(location: &str, limit: u64, label: &str) -> Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    let reader = crate::bucket_update::source_reader(location)
        .with_context(|| format!("cannot read {label} at {location}"))?;
    reader
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("cannot read {label} at {location}"))?;
    if bytes.len() as u64 > limit {
        return Ok(None);
    }
    Ok(Some(bytes))
}

fn read_json<T: DeserializeOwned>(location: &str, limit: u64, label: &str) -> Result<T> {
    let bytes = read_limited(location, limit, label)?
        .with_context(|| format!("{label} at {location} exceeds {limit} bytes"))?;
    serde_json::from_slice(&bytes).with_context(|| format!("invalid {label} at {location}"))
}

fn read_requirement(location: &str, tag: &str) -> Result<Option<Requirement>> {
    let Some(bytes) = read_limited(location, REQUIRES_MAX_BYTES, REQUIRES_ASSET)? else {
        eprintln!(
            "Skipping main release {tag}: {REQUIRES_ASSET} at {location} exceeds {REQUIRES_MAX_BYTES} bytes."
        );
        return Ok(None);
    };
    let requires: Requires = match serde_json::from_slice(&bytes) {
        Ok(requires) => requires,
        Err(error) => {
            let error = anyhow::Error::new(error)
                .context(format!("invalid {REQUIRES_ASSET} at {location}"));
            eprintln!("Skipping main release {tag}: {error:#}");
            return Ok(None);
        }
    };
    let minimum_text = requires.min_modn_version.trim().to_owned();
    let minimum = match semver::Version::parse(&minimum_text) {
        Ok(minimum) => minimum,
        Err(error) => {
            let error = anyhow::Error::new(error)
                .context(format!("invalid min_modn_version {minimum_text:?}"));
            eprintln!("Skipping main release {tag}: {error:#}");
            return Ok(None);
        }
    };
    Ok(Some(Requirement {
        schema_version: requires.schema_version,
        minimum,
        minimum_text,
    }))
}

fn asset_url<'a>(release: &'a Release, name: &str) -> Option<&'a str> {
    release
        .assets
        .iter()
        .find(|asset| asset.name == name)
        .map(|asset| asset.browser_download_url.as_str())
}

fn selected(release: &Release, zip: &str, newer: Option<UnsupportedRelease>) -> Result<Selection> {
    let tag = release.tag_name.as_str();
    let signature = asset_url(release, SIGNATURE_ASSET)
        .with_context(|| format!("main release {tag} has no {SIGNATURE_ASSET} asset"))?;
    Ok(Selection {
        tag: tag.to_owned(),
        zip: zip.to_owned(),
        signature: signature.to_owned(),
        newer,
    })
}

/// Read the release list and return the newest release that this build reads.
pub(super) fn select() -> Result<Selection> {
    let location = modn_core::buckets::main_release_list_override()
        .unwrap_or_else(|| DEFAULT_RELEASES.to_owned());
    let releases: Vec<Release> = read_json(&location, RELEASE_LIST_MAX_BYTES, "main release list")?;
    let current_text = env!("CARGO_PKG_VERSION");
    let current =
        semver::Version::parse(current_text).context("this Modn build has an invalid version")?;
    let mut newer = None;
    for release in releases {
        if release.draft || release.prerelease {
            continue;
        }
        let tag = release.tag_name.as_str();
        if tag.is_empty() {
            continue;
        }
        let Some(zip) = asset_url(&release, ZIP_ASSET) else {
            eprintln!("Skipping main release {tag}: no {ZIP_ASSET} asset.");
            continue;
        };
        let Some(requires) = asset_url(&release, REQUIRES_ASSET) else {
            // A legacy release carries no requirement asset. The install
            // checks still refuse a bucket that this build cannot read.
            return selected(&release, zip, newer);
        };
        let Some(requirement) = read_requirement(requires, tag)? else {
            continue;
        };
        if requirement.schema_version == SUPPORTED_SCHEMA_VERSION && requirement.minimum <= current
        {
            return selected(&release, zip, newer);
        }
        if newer.is_none() {
            newer = Some(UnsupportedRelease {
                tag: tag.to_owned(),
                min_modn_version: requirement.minimum_text,
                schema_version: requirement.schema_version,
            });
        }
    }
    match newer {
        Some(newer) => bail!(
            "no main bucket release is supported by Modn {current_text}; main release {} {}. Update Modn to get it.",
            newer.tag,
            newer.needs()
        ),
        None => bail!(
            "no main bucket release is supported by Modn {current_text}; no release has a readable {REQUIRES_ASSET}"
        ),
    }
}
