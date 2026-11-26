//! A release archive digest and how Modn knows it.
//!
//! A bucket manifest may pin an archive digest. A Git release has no pin
//! until Modn fetches the commit and hashes the packed archive. Keeping those
//! cases distinct prevents an empty string from standing in for a digest.

use anyhow::{bail, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// A BLAKE3 archive digest: 64 lowercase hexadecimal characters.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ArchiveDigest(String);

impl ArchiveDigest {
    /// Parse a BLAKE3 archive digest.
    ///
    /// # Errors
    ///
    /// Returns an error unless `value` is exactly 64 lowercase hexadecimal
    /// characters.
    pub fn new(value: &str) -> Result<Self> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            bail!("archive BLAKE3 digest must be 64 lowercase hex characters");
        }
        Ok(Self(value.to_owned()))
    }

    /// The digest as 64 lowercase hexadecimal characters.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ArchiveDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// The BLAKE3 digest of a release archive, and how Modn knows it.
///
/// Serialized declarations store a known digest as hexadecimal text and an
/// unknown digest as the empty string, so records written before this type
/// existed still decode. A stored known digest decodes as
/// [`ReleaseDigest::Pinned`] because history does not record which operation
/// computed it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ReleaseDigest {
    /// The catalog manifest pins this digest.
    Pinned(ArchiveDigest),
    /// Modn computed this digest after fetching and hashing the archive.
    Computed(ArchiveDigest),
    /// Modn has no digest yet: a Git release before Modn fetches it.
    Unknown,
}

impl ReleaseDigest {
    /// Record a BLAKE3 pin published by a catalog manifest.
    ///
    /// # Errors
    ///
    /// Returns an error unless `value` is exactly 64 lowercase hexadecimal
    /// characters.
    pub fn pinned(value: &str) -> Result<Self> {
        Ok(Self::Pinned(ArchiveDigest::new(value)?))
    }

    /// Record a BLAKE3 digest that Modn computed after a fetch.
    ///
    /// # Errors
    ///
    /// Returns an error unless `value` is exactly 64 lowercase hexadecimal
    /// characters.
    pub fn computed(value: &str) -> Result<Self> {
        Ok(Self::Computed(ArchiveDigest::new(value)?))
    }

    /// Use a catalog pin when the manifest publishes one.
    ///
    /// A missing pin is [`ReleaseDigest::Unknown`]. An empty or malformed
    /// value is an error, because a present pin is never empty.
    ///
    /// # Errors
    ///
    /// Returns an error when `pin` is `Some` and not a BLAKE3 digest.
    pub fn from_pin(pin: Option<&str>) -> Result<Self> {
        match pin {
            Some(value) => Self::pinned(value),
            None => Ok(Self::Unknown),
        }
    }

    /// The digest value, or `None` when it is not yet known.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Pinned(digest) | Self::Computed(digest) => Some(digest.as_str()),
            Self::Unknown => None,
        }
    }

    /// True when Modn has no digest for the release yet.
    #[must_use]
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }

    /// The storage form: hexadecimal text, or empty for an unknown digest.
    pub(crate) fn stored(&self) -> &str {
        match self {
            Self::Pinned(digest) | Self::Computed(digest) => digest.as_str(),
            Self::Unknown => "",
        }
    }
}

impl PartialEq<str> for ReleaseDigest {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == Some(other)
    }
}

impl PartialEq<&str> for ReleaseDigest {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == Some(*other)
    }
}

impl PartialEq<String> for ReleaseDigest {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == Some(other.as_str())
    }
}

impl Serialize for ReleaseDigest {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.stored())
    }
}

impl<'de> Deserialize<'de> for ReleaseDigest {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value.is_empty() {
            return Ok(Self::Unknown);
        }
        ArchiveDigest::new(&value)
            .map(Self::Pinned)
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests;
