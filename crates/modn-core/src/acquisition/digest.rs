use anyhow::Result;
use blake3::Hasher;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;

pub(super) fn copy_blake3(body: impl Read, output: &mut File, limit: u64) -> Result<(u64, String)> {
    let mut reader = body.take(limit + 1);
    let mut hasher = Hasher::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut total = 0;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        output.write_all(&buffer[..read])?;
        total += read as u64;
    }
    Ok((total, hasher.finalize().to_hex().to_string()))
}

pub(super) fn file_blake3(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Hasher::new();
    io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finalize().to_hex().to_string())
}

pub(super) fn file_has_blake3(path: &Path, blake3: &str) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(file_blake3(path)? == blake3),
        _ => Ok(false),
    }
}
