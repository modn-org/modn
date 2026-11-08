use anyhow::{bail, Context, Result};
use blake3::Hasher;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const CATALOG_DIGEST_DOMAIN: &str = "modn-catalog-blake3/v1";
const CATALOG_SIGNATURE_FILES: [&str; 2] = ["catalog.sig", "catalog.pub"];

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
        let mut content = fs::File::open(path)?;
        digest.update(name.as_bytes());
        digest.update([0]);
        digest.update(content.metadata()?.len().to_le_bytes());
        io::copy(&mut content, &mut digest)?;
    }
    Ok(format!("{:x}", digest.finalize()))
}

/// Canonical BLAKE3 digest of a catalog tree, displayed as `blake3:<hex>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogDigest(String);

impl CatalogDigest {
    /// Return the digest in `blake3:<lowercase hex>` form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CatalogDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn catalog_files(
    root: &Path,
    directory: &Path,
    is_root: bool,
    output: &mut Vec<(String, PathBuf)>,
) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        if is_root && name == OsStr::new(".git") {
            bail!("catalog root contains a .git entry");
        }
        let metadata = fs::symlink_metadata(&path)?;
        if is_link_or_reparse(&metadata) {
            bail!("non-regular catalog entry: {}", path.display());
        }
        if is_root
            && metadata.is_file()
            && CATALOG_SIGNATURE_FILES
                .iter()
                .any(|excluded| name == OsStr::new(excluded))
        {
            continue;
        }
        if metadata.is_dir() {
            catalog_files(root, &path, false, output)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .with_context(|| format!("catalog path outside root: {}", path.display()))?;
            let name = relative
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("non-UTF-8 catalog path: {}", path.display()))?;
            output.push((name.replace('\\', "/"), path));
        } else {
            bail!("non-regular catalog entry: {}", path.display());
        }
    }
    Ok(())
}

pub(crate) fn reject_case_collisions(entries: &[(String, PathBuf)]) -> Result<()> {
    let mut folded = BTreeMap::new();
    for (name, _) in entries {
        if let Some(previous) = folded.insert(name.to_lowercase(), name.as_str()) {
            bail!("catalog paths collide case-insensitively: {previous} and {name}");
        }
    }
    Ok(())
}

/// Hash a catalog tree with BLAKE3 in the canonical signature framing.
///
/// The digest covers the domain prefix, one NUL, then each regular file in
/// byte-wise relative-path order: path, NUL, little-endian u64 size, contents.
/// Root-level `catalog.sig` and `catalog.pub` files are excluded.
pub fn catalog_digest(root: &Path) -> Result<CatalogDigest> {
    let mut entries = Vec::new();
    catalog_files(root, root, true, &mut entries)?;
    reject_case_collisions(&entries)?;
    entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut digest = Hasher::new();
    digest.update(CATALOG_DIGEST_DOMAIN.as_bytes());
    digest.update(&[0]);
    for (name, path) in entries {
        let mut content = fs::File::open(&path)
            .with_context(|| format!("cannot open catalog file: {}", path.display()))?;
        digest.update(name.as_bytes());
        digest.update(&[0]);
        digest.update(&content.metadata()?.len().to_le_bytes());
        io::copy(&mut content, &mut digest)?;
    }
    Ok(CatalogDigest(format!(
        "blake3:{}",
        digest.finalize().to_hex()
    )))
}
