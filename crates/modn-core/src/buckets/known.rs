//! Optional `known-buckets.json` in the official `main` bucket root.
//!
//! The file lists community buckets that Modn can offer by short name. The
//! `main` catalog signature covers it, because the catalog digest covers every
//! root file except `catalog.sig` and `catalog.pub`. Listing a bucket is
//! metadata only: it neither registers the bucket nor trusts its modules.

use super::manifest::valid_id;
use super::signature::CatalogKeyId;
use super::store::BucketStore;
use anyhow::{bail, Context, Result};
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::Path;

/// File name of the optional bucket list in the `main` root.
pub(super) const KNOWN_BUCKETS_FILE: &str = "known-buckets.json";
const SCHEMA_VERSION: u32 = 1;
/// Largest accepted `known-buckets.json` file.
const MAX_FILE_BYTES: u64 = 256 * 1024;
/// Largest number of listed buckets.
const MAX_ENTRIES: usize = 256;
const MAX_SOURCE_CHARS: usize = 2048;
const MAX_REVISION_CHARS: usize = 255;
const MAX_DESCRIPTION_CHARS: usize = 200;

/// How Modn would fetch one listed community bucket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum KnownSourceKind {
    /// A signed Git catalog fetched from an HTTPS remote.
    Git,
    /// A signed catalog ZIP downloaded from an HTTPS URL.
    Zip,
}

impl KnownSourceKind {
    /// Return the lowercase source kind used by the file format.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Git => "git",
            Self::Zip => "zip",
        }
    }
}

/// One community bucket listed by the official `main` bucket.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct KnownBucket {
    /// Short bucket name that a later sync would use.
    pub name: String,
    /// How Modn would fetch the bucket.
    pub kind: KnownSourceKind,
    /// HTTPS source URL.
    pub source: String,
    /// Git revision to fetch; absent for a `zip` source.
    pub revision: Option<String>,
    /// Minisign key identifier of the bucket's `catalog.pub`.
    pub fingerprint: CatalogKeyId,
    /// One-line description of the bucket.
    pub description: String,
}

/// Validated entries of one `known-buckets.json`, sorted by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct KnownBuckets {
    entries: Vec<KnownBucket>,
}

impl KnownBuckets {
    /// Parse and validate `known-buckets.json` bytes.
    ///
    /// Every refusal names the offending entry and field.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() as u64 > MAX_FILE_BYTES {
            bail!("{KNOWN_BUCKETS_FILE} exceeds the {MAX_FILE_BYTES}-byte size limit");
        }
        let file: RawFile = serde_json::from_slice(bytes)
            .map_err(|error| anyhow::anyhow!("invalid {KNOWN_BUCKETS_FILE}: {error}"))?;
        if file.schema_version != SCHEMA_VERSION {
            bail!(
                "{KNOWN_BUCKETS_FILE} uses schema_version {}; Modn supports {SCHEMA_VERSION}",
                file.schema_version
            );
        }
        if file.buckets.0.len() > MAX_ENTRIES {
            bail!(
                "{KNOWN_BUCKETS_FILE} lists {} buckets; the limit is {MAX_ENTRIES}",
                file.buckets.0.len()
            );
        }
        let mut entries = Vec::with_capacity(file.buckets.0.len());
        for (name, value) in file.buckets.0 {
            entries.push(entry(&name, value)?);
        }
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(Self { entries })
    }

    /// Return the entries in name order.
    #[must_use]
    pub fn entries(&self) -> &[KnownBucket] {
        &self.entries
    }

    /// Return one entry by short bucket name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&KnownBucket> {
        self.entries
            .binary_search_by(|entry| entry.name.as_str().cmp(name))
            .ok()
            .map(|index| &self.entries[index])
    }

    /// True when the official `main` bucket lists no community buckets.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of listed community buckets.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

impl BucketStore {
    /// Read the optional bucket list from the installed official `main` copy.
    ///
    /// The copy lives inside the selected `--state-dir` profile when one is
    /// selected. Returns an empty list when `known-buckets.json` is absent.
    pub fn known_buckets(&self) -> Result<KnownBuckets> {
        Self::check_registry_root(&self.roots.official_bucket_root())?;
        let path = self.roots.official_main().join(KNOWN_BUCKETS_FILE);
        let Some(bytes) = read_declared(&path)? else {
            return Ok(KnownBuckets::default());
        };
        KnownBuckets::parse(&bytes)
            .map_err(|error| anyhow::anyhow!("invalid {KNOWN_BUCKETS_FILE}: {error:#}"))
    }
}

/// Validate the optional `known-buckets.json` in a staged `main` tree.
pub(super) fn validate_root(root: &Path) -> Result<()> {
    match read_declared(&root.join(KNOWN_BUCKETS_FILE))? {
        None => Ok(()),
        Some(bytes) => KnownBuckets::parse(&bytes)
            .map(|_| ())
            .map_err(|error| anyhow::anyhow!("{KNOWN_BUCKETS_FILE} in the main bucket: {error:#}")),
    }
}

/// Read the optional file, or `None` when it is absent.
fn read_declared(path: &Path) -> Result<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(metadata) => metadata,
    };
    if !metadata.file_type().is_file() {
        bail!(
            "known bucket list must be a regular file: {}",
            path.display()
        );
    }
    if metadata.len() > MAX_FILE_BYTES {
        bail!(
            "{KNOWN_BUCKETS_FILE} exceeds the {MAX_FILE_BYTES}-byte size limit: {}",
            path.display()
        );
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

fn entry(name: &str, value: serde_json::Value) -> Result<KnownBucket> {
    if !valid_id(name) || name == "main" {
        bail!(
            "known bucket \"{name}\": invalid name; use lowercase ASCII letters, digits and dashes, at most 64 characters, starting with a letter, and not the reserved name main"
        );
    }
    let raw: RawBucket = serde_json::from_value(value)
        .map_err(|error| anyhow::anyhow!("known bucket \"{name}\": {error}"))?;
    let kind = match raw.kind.as_str() {
        "git" => KnownSourceKind::Git,
        "zip" => KnownSourceKind::Zip,
        other => bail!("known bucket \"{name}\": kind \"{other}\" is unknown; expected git or zip"),
    };
    validate_source(name, &raw.source)?;
    match (&raw.revision, kind) {
        (Some(revision), KnownSourceKind::Git) => validate_revision(name, revision)?,
        (Some(_), KnownSourceKind::Zip) => {
            bail!("known bucket \"{name}\": revision is not allowed for a zip source")
        }
        (None, _) => {}
    }
    let fingerprint = parse_fingerprint(name, &raw.fingerprint)?;
    validate_description(name, &raw.description)?;
    Ok(KnownBucket {
        name: name.to_owned(),
        kind,
        source: raw.source,
        revision: raw.revision,
        fingerprint,
        description: raw.description,
    })
}

fn parse_fingerprint(name: &str, text: &str) -> Result<CatalogKeyId> {
    let message = format!("known bucket \"{name}\": fingerprint is not a 16-digit minisign key ID");
    if text.len() != 16 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("{message}");
    }
    CatalogKeyId::parse_hex(text).context(message)
}

fn validate_source(name: &str, source: &str) -> Result<()> {
    if source.chars().count() > MAX_SOURCE_CHARS {
        bail!("known bucket \"{name}\": source exceeds {MAX_SOURCE_CHARS} characters");
    }
    if source.chars().any(char::is_whitespace) || source.chars().any(char::is_control) {
        bail!("known bucket \"{name}\": source must not contain whitespace or control characters");
    }
    let Some(rest) = source.strip_prefix("https://") else {
        bail!("known bucket \"{name}\": source must be an https:// URL");
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') || source.contains('\\') {
        bail!("known bucket \"{name}\": source must be an https:// URL without credentials");
    }
    Ok(())
}

fn validate_revision(name: &str, revision: &str) -> Result<()> {
    if revision.is_empty()
        || revision.chars().count() > MAX_REVISION_CHARS
        || revision.starts_with('-')
        || revision.chars().any(char::is_whitespace)
        || revision.chars().any(char::is_control)
    {
        bail!(
            "known bucket \"{name}\": revision must be 1 to {MAX_REVISION_CHARS} characters without whitespace or control characters and must not start with -"
        );
    }
    Ok(())
}

fn validate_description(name: &str, description: &str) -> Result<()> {
    let length = description.chars().count();
    if length == 0 || length > MAX_DESCRIPTION_CHARS || description.chars().any(char::is_control) {
        bail!(
            "known bucket \"{name}\": description must be 1 to {MAX_DESCRIPTION_CHARS} characters without control characters"
        );
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    schema_version: u32,
    buckets: RawBuckets,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBucket {
    kind: String,
    source: String,
    #[serde(default)]
    revision: Option<String>,
    fingerprint: String,
    description: String,
}

/// Entry pairs kept in file order so a duplicate name can be refused.
struct RawBuckets(Vec<(String, serde_json::Value)>);

impl<'de> Deserialize<'de> for RawBuckets {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct BucketsVisitor;

        impl<'de> Visitor<'de> for BucketsVisitor {
            type Value = RawBuckets;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a map of bucket names to entries")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut entries = Vec::new();
                let mut names = BTreeSet::new();
                while let Some((name, value)) = map.next_entry::<String, serde_json::Value>()? {
                    if !names.insert(name.clone()) {
                        return Err(de::Error::custom(format!(
                            "known bucket \"{name}\": listed twice"
                        )));
                    }
                    entries.push((name, value));
                }
                Ok(RawBuckets(entries))
            }
        }

        deserializer.deserialize_map(BucketsVisitor)
    }
}

#[cfg(test)]
#[path = "known_loader_tests.rs"]
mod loader_tests;
#[cfg(test)]
#[path = "known_test_support.rs"]
mod test_support;
#[cfg(test)]
#[path = "known_tests.rs"]
mod tests;
