use super::{read_array, section_end, seek, u32_at, u64_at, Compression};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::io::{Read, Seek};

const MAX_INDEX_BYTES: u64 = 16 * 1024 * 1024;

/// RDAR v12 structural summary. CRC, hashes, codecs and payloads are not verified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
#[expect(
    missing_docs,
    reason = "Fields name declared header values and index counts"
)]
pub struct RedengineMetadata {
    pub version: u32,
    pub declared_bytes: u64,
    pub index_offset: u64,
    pub index_bytes: u32,
    pub debug_offset: u64,
    pub debug_bytes: u32,
    /// Custom data starts after the 172-byte extended header; contents are not read.
    pub custom_data_bytes: u32,
    pub file_table_offset: u32,
    pub file_table_bytes: u32,
    /// Declared CRC64, reported without verification.
    pub index_crc64: u64,
    pub file_count: u32,
    pub segment_count: u32,
    pub dependency_count: u32,
    /// Segments whose stored and expanded lengths differ; this does not identify a codec.
    pub encoded_segment_count: u32,
    /// RDAR index records do not identify the compression codec.
    pub compression: Compression,
}

pub(super) fn inspect(reader: &mut (impl Read + Seek), bytes: u64) -> Result<RedengineMetadata> {
    let header = read_array::<36>(reader)?;
    let version = u32_at(&header, 0);
    if version != 12 {
        bail!("unsupported REDengine archive version {version} (expected 12)");
    }
    let index_offset = u64_at(&header, 4);
    let index_bytes = u32_at(&header, 12);
    let debug_offset = u64_at(&header, 16);
    let debug_bytes = u32_at(&header, 24);
    let declared_bytes = u64_at(&header, 28);
    if declared_bytes != bytes {
        bail!("REDengine declared size does not match file length");
    }
    let custom_data_bytes = u32::from_le_bytes(read_array::<4>(reader)?);
    let custom_end = section_end(172, u64::from(custom_data_bytes), bytes)?;
    if index_offset < custom_end || index_bytes < 28 || u64::from(index_bytes) > MAX_INDEX_BYTES {
        bail!("invalid or unsupported REDengine index size or offset");
    }
    let index_end = section_end(index_offset, u64::from(index_bytes), bytes)?;
    let debug_end = section_end(debug_offset, u64::from(debug_bytes), bytes)?;
    if debug_bytes != 0
        && (debug_offset < custom_end || (debug_offset < index_end && debug_end > index_offset))
    {
        bail!("REDengine debug section overlaps header or index");
    }
    seek(reader, index_offset)?;
    let index = read_array::<28>(reader)?;
    let file_table_offset = u32_at(&index, 0);
    let file_table_bytes = u32_at(&index, 4);
    let file_count = u32_at(&index, 16);
    let segment_count = u32_at(&index, 20);
    let dependency_count = u32_at(&index, 24);
    let records_bytes = u64::from(file_count)
        .checked_mul(56)
        .and_then(|n| n.checked_add(u64::from(segment_count).checked_mul(16)?))
        .and_then(|n| n.checked_add(u64::from(dependency_count).checked_mul(8)?))
        .context("REDengine table size overflow")?;
    if file_table_offset != 8
        || u64::from(file_table_bytes) != records_bytes + 20
        || u64::from(index_bytes) != records_bytes + 28
    {
        bail!("invalid or unsupported REDengine table layout or counts");
    }
    for _ in 0..file_count {
        let entry = read_array::<56>(reader)?;
        let inline = u32_at(&entry, 16);
        let start = u32_at(&entry, 20);
        let end = u32_at(&entry, 24);
        let dependency_start = u32_at(&entry, 28);
        let dependency_end = u32_at(&entry, 32);
        if start >= end
            || end > segment_count
            || inline > end - start
            || dependency_start > dependency_end
            || dependency_end > dependency_count
        {
            bail!("invalid REDengine file segment or dependency range");
        }
    }
    let mut encoded_segment_count = 0;
    for _ in 0..segment_count {
        let segment = read_array::<16>(reader)?;
        let offset = u64_at(&segment, 0);
        let stored = u32_at(&segment, 8);
        let expanded = u32_at(&segment, 12);
        let end = section_end(offset, u64::from(stored), bytes)?;
        if offset < custom_end
            || (offset < index_end && end > index_offset)
            || (debug_bytes != 0 && offset < debug_end && end > debug_offset)
        {
            bail!("REDengine payload segment overlaps metadata");
        }
        if stored != expanded {
            encoded_segment_count += 1;
        }
    }
    Ok(RedengineMetadata {
        version,
        declared_bytes,
        index_offset,
        index_bytes,
        debug_offset,
        debug_bytes,
        custom_data_bytes,
        file_table_offset,
        file_table_bytes,
        index_crc64: u64_at(&index, 8),
        file_count,
        segment_count,
        dependency_count,
        encoded_segment_count,
        compression: Compression::Unknown,
    })
}
