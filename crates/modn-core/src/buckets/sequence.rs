//! Signed catalog sequence numbers and expiry.
//!
//! A signed catalog may carry a root `sequence.json` file. The catalog digest
//! covers it, so `catalog.sig` binds the number and expiry exactly as it binds
//! every other catalog file. Modn stores the highest accepted number and its
//! digest per bucket, so a publisher increase is accepted, an equal number with
//! the same digest is an idempotent re-sync, and a lower, dropped or conflicting
//! number is refused.

use crate::error::{coded, ErrorCode};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Root file that carries the signed sequence number.
pub const SEQUENCE_FILE: &str = "sequence.json";
const MAX_SEQUENCE_BYTES: u64 = 4 * 1024;

/// The highest accepted `{sequence, digest}` for one bucket.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct StoredSequence {
    /// Accepted sequence number, at least 1.
    pub sequence: u64,
    /// Catalog digest of the accepted revision, as `blake3:<hex>`.
    pub digest: String,
}

impl StoredSequence {
    /// Construct one accepted record.
    #[must_use]
    pub fn new(sequence: u64, digest: impl Into<String>) -> Self {
        Self {
            sequence,
            digest: digest.into(),
        }
    }
}

/// A parsed `sequence.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedSequence {
    sequence: u64,
    expires: Option<String>,
    expires_unix: Option<u64>,
}

impl SignedSequence {
    /// The publisher sequence number, at least 1.
    #[must_use]
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    /// The raw expiry text, when the catalog names one.
    #[must_use]
    pub fn expires(&self) -> Option<&str> {
        self.expires.as_deref()
    }
}

/// Why a sync reports a warning without refusing the catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CatalogWarningKind {
    /// The signed catalog expired and still syncs.
    Expired,
}

/// One non-fatal warning carried by a sync result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct CatalogWarning {
    /// The warning class a front end can switch on.
    pub kind: CatalogWarningKind,
    /// Human-readable line for text output.
    pub message: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SequenceFile {
    schema_version: u32,
    sequence: u64,
    #[serde(default)]
    expires: Option<String>,
}

fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.file_type().is_file() {
        bail!("{SEQUENCE_FILE} must be a regular file: {}", path.display());
    }
    if metadata.len() > MAX_SEQUENCE_BYTES {
        bail!("{SEQUENCE_FILE} exceeds size limit: {}", path.display());
    }
    fs::read(path)
        .with_context(|| format!("cannot read {SEQUENCE_FILE}: {}", path.display()))
        .map(Some)
}

/// Read and strictly validate `sequence.json` from one catalog root.
///
/// Unknown fields, an unknown schema version, a zero number and an expiry that
/// is not `<YYYY-MM-DDTHH:MM:SSZ>` are refused.
pub(in crate::buckets) fn read_sequence(root: &Path) -> Result<Option<SignedSequence>> {
    let Some(bytes) = read_bounded(&root.join(SEQUENCE_FILE))? else {
        return Ok(None);
    };
    let file: SequenceFile = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid {SEQUENCE_FILE} in {}", root.display()))?;
    if file.schema_version != 1 {
        bail!(
            "{SEQUENCE_FILE} uses schema_version {}; Modn supports 1",
            file.schema_version
        );
    }
    if file.sequence == 0 {
        bail!("{SEQUENCE_FILE} sequence must be at least 1");
    }
    let expires_unix = file
        .expires
        .as_deref()
        .map(parse_expiry)
        .transpose()
        .with_context(|| format!("invalid expires in {SEQUENCE_FILE}"))?;
    Ok(Some(SignedSequence {
        sequence: file.sequence,
        expires: file.expires,
        expires_unix,
    }))
}

/// Compare a served signed sequence with the stored record.
///
/// Returns the record to store and any non-fatal warnings. A lower number or a
/// dropped number is [`ErrorCode::CatalogRollback`]; an equal number with a
/// different digest is [`ErrorCode::CatalogSequenceConflict`].
pub(in crate::buckets) fn accept_sequence(
    name: &str,
    stored: Option<&StoredSequence>,
    signed: Option<&SignedSequence>,
    digest: &str,
    now: u64,
) -> Result<(Option<StoredSequence>, Vec<CatalogWarning>)> {
    let Some(signed) = signed else {
        if let Some(stored) = stored {
            return Err(coded(
                ErrorCode::CatalogRollback,
                format!(
                    "Catalog {name} is unnumbered, but Modn already accepted signed sequence {}. A signed catalog cannot drop its sequence number; the rollback refused the sync and nothing was written. To accept this revision deliberately, remove the bucket and sync it again.",
                    stored.sequence
                ),
            ));
        }
        return Ok((None, Vec::new()));
    };
    if let Some(stored) = stored {
        if signed.sequence < stored.sequence {
            return Err(coded(
                ErrorCode::CatalogRollback,
                format!(
                    "Catalog {name} serves signed sequence {}, but Modn already accepted sequence {}. A signed catalog cannot go back to an older number; the rollback refused the sync and nothing was written.",
                    signed.sequence, stored.sequence
                ),
            ));
        }
        if signed.sequence == stored.sequence && stored.digest != digest {
            return Err(coded(
                ErrorCode::CatalogSequenceConflict,
                format!(
                    "Catalog {name} serves signed sequence {} with digest {digest}, but Modn already accepted that number with digest {}. A sequence number names one revision; the conflict refused the sync and nothing was written.",
                    signed.sequence, stored.digest
                ),
            ));
        }
    }
    let mut warnings = Vec::new();
    if let (Some(expires), Some(expires_unix)) = (&signed.expires, signed.expires_unix) {
        if now > expires_unix {
            warnings.push(CatalogWarning {
                kind: CatalogWarningKind::Expired,
                message: format!(
                    "catalog {name} signed sequence {} expired at {expires}; it still syncs, but the publisher should sign a newer revision.",
                    signed.sequence
                ),
            });
        }
    }
    Ok((
        Some(StoredSequence::new(signed.sequence, digest)),
        warnings,
    ))
}

/// Parse `<YYYY-MM-DDTHH:MM:SSZ>` into seconds since the Unix epoch.
///
/// Only the UTC `Z` form is accepted. The parser validates the calendar date,
/// so month and day values cannot address a wrong instant.
fn parse_expiry(text: &str) -> Result<u64> {
    let bytes = text.as_bytes();
    let shape = bytes.len() == 20
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[19] == b'Z';
    if !shape {
        bail!("expected a UTC timestamp such as 2026-12-31T00:00:00Z");
    }
    let digits = |range: std::ops::Range<usize>| -> Result<u32> {
        let slice = &bytes[range.clone()];
        if !slice.iter().all(u8::is_ascii_digit) {
            bail!("expected a UTC timestamp such as 2026-12-31T00:00:00Z");
        }
        Ok(slice
            .iter()
            .fold(0u32, |value, digit| value * 10 + u32::from(digit - b'0')))
    };
    let year = digits(0..4)?;
    let month = digits(5..7)?;
    let day = digits(8..10)?;
    let hour = digits(11..13)?;
    let minute = digits(14..16)?;
    let second = digits(17..19)?;
    if !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        bail!("timestamp is outside the calendar: {text}");
    }
    let days = days_from_civil(i64::from(year), month, day);
    let seconds = days * 86_400 + i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second);
    u64::try_from(seconds).context("timestamp is before the Unix epoch")
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days since 1970-01-01 for a Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let shifted_month = i64::from((month + 9) % 12);
    let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(day) - 1;
    let day_of_era =
        year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests;
