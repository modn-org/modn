//! Strict path and identity checks for uninstalling previously adopted records.
//! No new ownership is granted through this compatibility path.
use crate::planning::prohibited_payload;
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024 * 1024;

pub(crate) fn checked_legacy_file(root: &Path, relative: &str) -> Result<(PathBuf, u64, String)> {
    if relative.len() > 256
        || !relative.contains('/')
        || !crate::archive::validated_name(relative).is_ok_and(|name| name == relative)
        || prohibited_payload(relative)
    {
        bail!("invalid legacy managed file path: {relative}");
    }
    let mut path = root.to_path_buf();
    let parts: Vec<_> = relative.split('/').collect();
    for (index, part) in parts.iter().enumerate() {
        path.push(part);
        let meta = fs::symlink_metadata(&path)
            .with_context(|| format!("cannot inspect legacy managed path {}", path.display()))?;
        if meta.file_type().is_symlink()
            || (index + 1 == parts.len() && !meta.file_type().is_file())
            || (index + 1 < parts.len() && !meta.file_type().is_dir())
        {
            bail!(
                "linked, special or non-file legacy managed path: {}",
                path.display()
            );
        }
        let canonical = path.canonicalize()?;
        if !canonical.starts_with(root) {
            bail!("legacy managed path escapes installation root");
        }
        // Reject short-name aliases and case/Unicode alternate spellings.
        let exact = path
            .parent()
            .context("missing legacy managed parent")?
            .read_dir()?
            .filter_map(std::result::Result::ok)
            .any(|entry| entry.file_name() == std::ffi::OsStr::new(part));
        if !exact {
            bail!("aliased legacy managed path: {}", path.display());
        }
    }
    let before = fs::metadata(&path)?;
    if before.len() > MAX_FILE_BYTES {
        bail!("legacy managed file exceeds 4 GiB");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.nlink() != 1 {
            bail!("hard-linked files cannot be legacy managed");
        }
    }
    #[cfg(windows)]
    {
        let system_root =
            std::env::var_os("SystemRoot").context("cannot verify Windows hard-link count")?;
        let fsutil = PathBuf::from(system_root).join("System32/fsutil.exe");
        let output = std::process::Command::new(fsutil)
            .args(["hardlink", "list"])
            .arg(&path)
            .output()?;
        if !output.status.success() {
            bail!(
                "cannot verify Windows hard-link count for {}",
                path.display()
            );
        }
        let listing = String::from_utf8(output.stdout).context("invalid hard-link listing")?;
        if listing
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count()
            != 1
        {
            bail!("hard-linked files cannot be legacy managed");
        }
    }
    let identity = same_file::Handle::from_path(&path)?;
    let mut file = File::open(&path)?;
    let mut hash = Sha256::new();
    let bytes = io::copy(&mut (&mut file).take(before.len() + 1), &mut hash)?;
    if bytes != before.len()
        || fs::metadata(&path)?.len() != before.len()
        || same_file::Handle::from_path(&path)? != identity
    {
        bail!(
            "legacy managed file changed while reading: {}",
            path.display()
        );
    }
    Ok((path, bytes, format!("{:x}", hash.finalize())))
}
