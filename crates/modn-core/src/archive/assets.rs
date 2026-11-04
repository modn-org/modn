//! Bounded, read-only inspection of asset structures, without game or install policy.

mod dds;
mod redengine;
#[cfg(test)]
mod tests;

pub use dds::{DdsMetadata, Dx10Metadata, PixelFormat};
pub use redengine::RedengineMetadata;

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Structural metadata only. Payload integrity and game compatibility are not verified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Inspection {
    /// Actual file length from the opened handle, in bytes.
    pub bytes: u64,
    /// Format selected by signature, independent of the filename.
    #[serde(flatten)]
    pub format: AssetFormat,
}

/// Supported structural subsets. Unknown signatures and versions return errors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "format", content = "metadata", rename_all = "snake_case")]
#[non_exhaustive]
pub enum AssetFormat {
    /// Legacy DDS header, with its DX10 extension when present.
    Dds(DdsMetadata),
    /// REDengine RDAR version 12 header and bounded index records.
    Redengine(RedengineMetadata),
}

/// Compression identified only from a documented format identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
#[expect(
    missing_docs,
    reason = "Variant names identify standard compression formats"
)]
pub enum Compression {
    Uncompressed,
    Bc1,
    Bc2,
    Bc3,
    Bc4,
    Bc5,
    Bc6h,
    Bc7,
    /// The header does not identify a codec supported by this inspector.
    Unknown,
}

/// Inspect DDS or RDAR structure without decoding payloads or changing files.
///
/// Reads at most 148 DDS header bytes or 16 MiB of REDengine index metadata,
/// plus 44 header bytes. Memory use does not grow with asset size or counts.
/// The caller selects the path. No game identity or installation authority follows.
/// Concurrent file changes are not prevented; this is not an integrity snapshot.
///
/// # Errors
/// Returns an error for I/O failures, non-files, unknown formats, unsupported
/// versions, malformed headers, or index sections outside the file or budget.
pub fn inspect(path: &Path) -> Result<Inspection> {
    if !path.metadata()?.is_file() {
        bail!("asset input must be a regular file");
    }
    let mut file = File::open(path).context("cannot open asset for read-only inspection")?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        bail!("asset input must be a regular file");
    }
    inspect_reader(&mut file, metadata.len())
}

fn inspect_reader(reader: &mut (impl Read + Seek), bytes: u64) -> Result<Inspection> {
    let signature = read_array::<4>(reader)?;
    let format = match &signature {
        b"DDS " => AssetFormat::Dds(dds::inspect(reader)?),
        b"RDAR" => AssetFormat::Redengine(redengine::inspect(reader, bytes)?),
        _ => bail!("unsupported asset signature (expected DDS or RDAR)"),
    };
    Ok(Inspection { bytes, format })
}

fn read_array<const N: usize>(reader: &mut impl Read) -> Result<[u8; N]> {
    let mut bytes = [0; N];
    reader
        .read_exact(&mut bytes)
        .context("truncated asset structure")?;
    Ok(bytes)
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("fixed header field"),
    )
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("fixed header field"),
    )
}

fn section_end(offset: u64, length: u64, bytes: u64) -> Result<u64> {
    let end = offset
        .checked_add(length)
        .context("asset section overflow")?;
    if end > bytes {
        bail!("asset section extends outside the file");
    }
    Ok(end)
}

fn seek(reader: &mut impl Seek, offset: u64) -> Result<()> {
    reader.seek(SeekFrom::Start(offset))?;
    Ok(())
}
