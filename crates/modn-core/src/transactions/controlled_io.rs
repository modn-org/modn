//! Bounded copy loops that honor the operation control inside one large file.
//!
//! A cancellation before the commit point stops the copy or hash of one
//! archive member or one owned file after at most one chunk (#176).

use crate::progress::OperationControl;
use anyhow::Result;
use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::path::Path;

const CHUNK_BYTES: usize = 64 * 1024;

/// Copy at most `limit` bytes from `input` to `output`.
///
/// The loop checks `control` before each chunk, so a cancellation before
/// the commit point returns [`crate::progress::Cancelled`].
pub(super) fn copy(
    input: &mut (impl Read + ?Sized),
    output: &mut impl Write,
    limit: u64,
    control: &OperationControl,
) -> Result<u64> {
    let mut buffer = vec![0u8; CHUNK_BYTES];
    let mut copied = 0u64;
    while copied < limit {
        control.checkpoint()?;
        let want = (limit - copied).min(CHUNK_BYTES as u64) as usize;
        let read = match input.read(&mut buffer[..want]) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        output.write_all(&buffer[..read])?;
        copied += read as u64;
    }
    Ok(copied)
}

/// The BLAKE3 digest of the file at `path`, checking `control` per chunk.
pub(super) fn digest_file(path: &Path, control: &OperationControl) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    copy(&mut File::open(path)?, &mut hasher, u64::MAX, control)?;
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
#[path = "controlled_io_tests.rs"]
mod tests;
