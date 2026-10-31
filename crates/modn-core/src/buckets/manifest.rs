use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// One immutable package version, independent of installer policy.
#[expect(
    missing_docs,
    reason = "Serialized schema fields directly specify their purpose"
)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PackageManifest {
    pub schema_version: u32,
    pub id: String,
    pub game: String,
    pub name: String,
    pub version: String,
    pub archive_format: ArchiveFormat,
    pub sha256: String,
    pub source: PackageSource,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
}

/// Format of the original, unextracted download.
#[expect(
    missing_docs,
    reason = "Serialized schema fields directly specify their purpose"
)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum ArchiveFormat {
    Zip,
    #[serde(rename = "7z")]
    SevenZ,
}

/// Metadata only: no URLs are fetched by the bucket foundation.
#[expect(
    missing_docs,
    reason = "Serialized schema fields directly specify their purpose"
)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
#[non_exhaustive]
pub enum PackageSource {
    Manual { filename: String },
    Https { url: String },
}

pub(super) fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !value.ends_with('-')
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
    pub(super) fn validate(&self, game: &str, id: &str) -> Result<()> {
        if self.schema_version != 1 {
            bail!("unsupported bucket package schema");
        }
        if !valid_id(&self.id) || !valid_id(&self.game) || self.id != id || self.game != game {
            bail!("bucket package identity disagrees with games/<game>/<id>.json path");
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
        if self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            bail!("package sha256 must be 64 lowercase hex characters");
        }
        match &self.source {
            PackageSource::Manual { filename } => {
                let extension = match self.archive_format {
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
            PackageSource::Https { url } if !valid_https(url) => {
                bail!("download URL must be HTTPS")
            }
            _ => {}
        }
        if self.homepage.as_ref().is_some_and(|url| !valid_https(url)) {
            bail!("homepage URL must be HTTPS");
        }
        let mut unique = HashSet::new();
        for dependency in &self.dependencies {
            if !valid_id(dependency) || dependency == &self.id || !unique.insert(dependency) {
                bail!("invalid, self-referential or duplicate dependency");
            }
        }
        Ok(())
    }
}
