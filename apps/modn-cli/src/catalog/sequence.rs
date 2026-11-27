//! Reading and writing the signed `sequence.json` catalog file.
//!
//! The catalog digest covers this root file, so `catalog.sig` binds the number
//! and the optional expiry. Modn core validates the same file when it syncs a
//! catalog; the authoring commands read and write it here so a publisher can
//! see and control the signed number.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const SEQUENCE_FILE: &str = "sequence.json";
/// Root file that carries a rotation statement.
pub(crate) const ROTATION_FILE: &str = "rotation.json";
/// Root file that carries a revocation statement.
pub(crate) const REVOCATION_FILE: &str = "revocation.json";
const MAX_SEQUENCE_BYTES: u64 = 4 * 1024;
const MAX_STATEMENT_BYTES: u64 = 32 * 1024;

/// One parsed `sequence.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SequenceFile {
    pub(crate) sequence: u64,
    pub(crate) expires: Option<String>,
}

/// The values a sign run writes to `sequence.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SequencePlan {
    pub(crate) sequence: u64,
    pub(crate) expires: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SequenceInput {
    schema_version: u32,
    sequence: u64,
    #[serde(default)]
    expires: Option<String>,
}

#[derive(Serialize)]
struct SequenceOutput<'a> {
    schema_version: u32,
    sequence: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires: Option<&'a str>,
}

/// Read and validate `<root>/sequence.json`, or report that it is absent.
pub(crate) fn read(root: &Path) -> Result<Option<SequenceFile>> {
    let path = root.join(SEQUENCE_FILE);
    let metadata = match fs::symlink_metadata(&path) {
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
    let bytes = fs::read(&path)
        .with_context(|| format!("cannot read {SEQUENCE_FILE}: {}", path.display()))?;
    let file: SequenceInput = serde_json::from_slice(&bytes)
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
    if let Some(expires) = &file.expires {
        parse_expiry(expires).with_context(|| format!("invalid expires in {SEQUENCE_FILE}"))?;
    }
    Ok(Some(SequenceFile {
        sequence: file.sequence,
        expires: file.expires,
    }))
}

/// Decide the number and expiry that `modn catalog sign` writes.
///
/// A published rotation or revocation statement names the revision that must
/// carry it, so its number is a floor: `sign` never signs a revision older than
/// a statement in the tree. `None` means the run leaves `sequence.json`
/// untouched. A numbered file that already exists may stay, rise, or keep its
/// number with a new expiry; it never drops.
pub(crate) fn plan(
    existing: Option<&SequenceFile>,
    statement: Option<u64>,
    flag: Option<&str>,
    expires: Option<&str>,
) -> Result<Option<SequencePlan>> {
    if let Some(expires) = expires {
        require_future(expires)?;
    }
    let file_number = existing.map(|file| file.sequence);
    let floor = highest(file_number, statement);
    let number = match flag {
        None => match (floor, expires) {
            (None, None) => return Ok(None),
            (None, Some(_)) => bail!(
                "`--expires` needs `--sequence`, an existing {SEQUENCE_FILE} or a numbered key statement; run `modn catalog sign <dir> --sequence next --expires <time>`"
            ),
            (Some(number), None) => {
                if file_number.is_some_and(|file| number <= file) {
                    return Ok(None);
                }
                number
            }
            (Some(number), Some(_)) => number,
        },
        Some("next") => {
            let after_file = match file_number {
                Some(file) => file.checked_add(1).ok_or_else(|| {
                    anyhow::anyhow!("{SEQUENCE_FILE} sequence cannot rise above u64")
                })?,
                None => 1,
            };
            match statement {
                Some(statement) => after_file.max(statement),
                None => after_file,
            }
        }
        Some(text) => {
            let number = parse_number(text)?;
            if let Some(file) = file_number {
                if number < file {
                    bail!(
                        "{SEQUENCE_FILE} already holds sequence {file}, so this run cannot write sequence {number}; a signed catalog number never goes down"
                    );
                }
            }
            if let Some(statement) = statement {
                if number < statement {
                    bail!(
                        "a published key statement names sequence {statement}, so this run cannot sign sequence {number}; sign sequence {statement} or higher, or remove the statement before signing an older revision"
                    );
                }
            }
            number
        }
    };
    let written_expires = match flag {
        Some(_) => expires.map(str::to_owned),
        None => expires
            .map(str::to_owned)
            .or_else(|| existing.and_then(|file| file.expires.clone())),
    };
    Ok(Some(SequencePlan {
        sequence: number,
        expires: written_expires,
    }))
}

fn highest(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (left, right) => left.or(right),
    }
}

/// The highest sequence that a published key statement names, if any.
///
/// `sign` treats it as a floor, because a revision must never be older than a
/// statement inside it. A statement without a number, or no statement at all,
/// adds no floor. The file is read as data only; sync still validates the
/// statement itself.
pub(crate) fn published_statement_sequence(root: &Path) -> Result<Option<u64>> {
    let mut highest = None;
    for file in [ROTATION_FILE, REVOCATION_FILE] {
        let Some(value) = read_statement(root, file)? else {
            continue;
        };
        let Some(number) = value.get("sequence").and_then(serde_json::Value::as_u64) else {
            continue;
        };
        if number == 0 {
            bail!("{file} names sequence 0; a key statement sequence must be at least 1");
        }
        highest = Some(highest.map_or(number, |current: u64| current.max(number)));
    }
    Ok(highest)
}

fn read_statement(root: &Path, file: &str) -> Result<Option<serde_json::Value>> {
    let path = root.join(file);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.file_type().is_file() {
        bail!("{file} must be a regular file: {}", path.display());
    }
    if metadata.len() > MAX_STATEMENT_BYTES {
        bail!("{file} exceeds size limit: {}", path.display());
    }
    let bytes =
        fs::read(&path).with_context(|| format!("cannot read {file}: {}", path.display()))?;
    let value = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid {file} in {}", root.display()))?;
    Ok(Some(value))
}

/// The statement sequence for `rotate` or `revoke`.
///
/// A statement describes the revision that will carry it, so a numbered catalog
/// names the next number: the file's number plus one. This command never writes
/// `sequence.json`; the following `sign` signs that number and writes it. An
/// explicit number must be at least the next number. An unnumbered catalog
/// yields no number unless `--sequence` names one, which numbers the catalog at
/// the following `sign`.
pub(crate) fn statement_sequence(root: &Path, explicit: Option<u64>) -> Result<Option<u64>> {
    let existing = read(root)?;
    match (explicit, existing) {
        (Some(number), Some(file)) => {
            let minimum = next_number(file.sequence)?;
            if number < minimum {
                bail!(
                    "--sequence {number} is not after {SEQUENCE_FILE} sequence {}; a key statement describes the next revision, so use at least {minimum} or omit --sequence",
                    file.sequence
                );
            }
            Ok(Some(number))
        }
        (Some(number), None) => {
            if number == 0 {
                bail!("--sequence must be at least 1");
            }
            Ok(Some(number))
        }
        (None, Some(file)) => Ok(Some(next_number(file.sequence)?)),
        (None, None) => Ok(None),
    }
}

fn next_number(number: u64) -> Result<u64> {
    number
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("{SEQUENCE_FILE} sequence cannot rise above u64"))
}

/// Write `<root>/sequence.json` atomically from the plan.
pub(crate) fn write(root: &Path, plan: &SequencePlan) -> Result<()> {
    let body = serde_json::to_vec(&SequenceOutput {
        schema_version: 1,
        sequence: plan.sequence,
        expires: plan.expires.as_deref(),
    })
    .context("cannot build sequence.json")?;
    super::keys::write_signature(&root.join(SEQUENCE_FILE), &body)
}

fn parse_number(text: &str) -> Result<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("--sequence must be a number of at least 1, or the word next: {text}");
    }
    let number: u64 = text
        .parse()
        .with_context(|| format!("--sequence is not a number: {text}"))?;
    if number == 0 {
        bail!("--sequence must be at least 1");
    }
    Ok(number)
}

/// Refuse an expiry that is not strictly after the current UTC time.
pub(crate) fn require_future(expires: &str) -> Result<()> {
    let at = parse_expiry(expires)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_secs();
    if at <= now {
        bail!("--expires must be in the future: {expires} is not after the current UTC time");
    }
    Ok(())
}

/// Parse `<YYYY-MM-DDTHH:MM:SSZ>` into seconds since the Unix epoch.
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
        bail!("expected a UTC timestamp such as 2027-12-31T00:00:00Z");
    }
    let digits = |range: std::ops::Range<usize>| -> Result<u32> {
        let slice = &bytes[range];
        if !slice.iter().all(u8::is_ascii_digit) {
            bail!("expected a UTC timestamp such as 2027-12-31T00:00:00Z");
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
    let seconds =
        days * 86_400 + i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second);
    u64::try_from(seconds).context("timestamp is before the Unix epoch")
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
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
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}
