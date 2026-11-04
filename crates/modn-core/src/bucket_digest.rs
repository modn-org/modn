use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

fn files(root: &Path, directory: &Path, output: &mut Vec<(String, PathBuf)>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            files(root, &path, output)?;
        } else if kind.is_file() {
            let relative = path.strip_prefix(root).map_err(io::Error::other)?;
            let name = relative
                .to_str()
                .ok_or_else(|| io::Error::other("non-UTF-8 bucket path"))?;
            output.push((name.replace('\\', "/"), path));
        } else {
            return Err(io::Error::other(format!(
                "non-regular bucket entry: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

/// Hash regular files in byte-wise relative-path order.
/// Each file contributes its UTF-8 path, NUL, little-endian u64 size, then contents.
pub fn directory_digest(root: &Path) -> io::Result<String> {
    let mut entries = Vec::new();
    files(root, root, &mut entries)?;
    entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut digest = Sha256::new();
    for (name, path) in entries {
        let content = fs::read(path)?;
        digest.update(name.as_bytes());
        digest.update([0]);
        digest.update((content.len() as u64).to_le_bytes());
        digest.update(&content);
    }
    Ok(format!("{:x}", digest.finalize()))
}
