//! Bounded, link-refusing reads of theme files.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

/// Why a theme file could not be read.
#[derive(Debug)]
pub(super) enum ReadFailure {
    /// The path is a symbolic link or another reparse point.
    Link,
    /// The path is not a regular file.
    NotFile,
    /// The file is larger than the requested limit.
    TooLarge,
    /// The operating system refused the read.
    Io(io::Error),
}

impl fmt::Display for ReadFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Link => write!(f, "the path is a link"),
            Self::NotFile => write!(f, "the path is not a regular file"),
            Self::TooLarge => write!(f, "the file is too large"),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

/// Read at most `limit` bytes of a regular file without following links.
pub(super) fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, ReadFailure> {
    let metadata = fs::symlink_metadata(path).map_err(ReadFailure::Io)?;
    if metadata.file_type().is_symlink() {
        return Err(ReadFailure::Link);
    }
    if !metadata.is_file() {
        return Err(ReadFailure::NotFile);
    }
    if metadata.len() > limit {
        return Err(ReadFailure::TooLarge);
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(ReadFailure::Io)?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(ReadFailure::Io)?;
    if bytes.len() as u64 > limit {
        return Err(ReadFailure::TooLarge);
    }
    Ok(bytes)
}

/// Read at most `limit` leading bytes of a regular file without following links.
pub(super) fn read_prefix(path: &Path, limit: usize) -> Result<Vec<u8>, ReadFailure> {
    let metadata = fs::symlink_metadata(path).map_err(ReadFailure::Io)?;
    if metadata.file_type().is_symlink() {
        return Err(ReadFailure::Link);
    }
    if !metadata.is_file() {
        return Err(ReadFailure::NotFile);
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(ReadFailure::Io)?
        .take(limit as u64)
        .read_to_end(&mut bytes)
        .map_err(ReadFailure::Io)?;
    Ok(bytes)
}
