use super::{AllowedRoots, Budget};
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Component, Path, PathBuf};

const MAX_RELATIVE_BYTES: usize = 256;
const MAX_DEPTH: usize = 16;

/// Whether the empty relative path may name an authorized root itself.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EmptyPath {
    /// The empty string names the root, for read-only top-level listing only.
    Root,
    /// The empty string is not a file or entry path and is rejected.
    Rejected,
}

pub(super) fn relative_components(value: &str, empty: EmptyPath) -> Result<Vec<&str>> {
    if value.is_empty() {
        return match empty {
            EmptyPath::Root => Ok(Vec::new()),
            EmptyPath::Rejected => bail!("unsafe relative path"),
        };
    }
    if value.len() > MAX_RELATIVE_BYTES
        || value.contains('\\')
        || value.contains(':')
        || value.contains('\0')
    {
        bail!("unsafe relative path");
    }
    let parts: Vec<_> = value.split('/').collect();
    if parts.len() > MAX_DEPTH
        || parts.iter().any(|part| {
            part.is_empty()
                || *part == "."
                || *part == ".."
                || part.ends_with([' ', '.'])
                || part.chars().any(char::is_control)
        })
    {
        bail!("unsafe relative path");
    }
    let path = Path::new(value);
    if path
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!("unsafe relative path");
    }
    Ok(parts)
}

pub(super) fn safe_target(
    root: &Path,
    relative: &str,
    empty: EmptyPath,
) -> Result<(PathBuf, fs::Metadata)> {
    let parts = relative_components(relative, empty)?;
    let mut current = root.to_path_buf();
    for (index, part) in parts.iter().enumerate() {
        current.push(part);
        let metadata = fs::symlink_metadata(&current)
            .with_context(|| format!("cannot inspect requested path {relative:?}"))?;
        if metadata.file_type().is_symlink()
            || (!metadata.file_type().is_file() && !metadata.file_type().is_dir())
        {
            bail!("linked or special paths are not allowed");
        }
        if index + 1 < parts.len() && !metadata.is_dir() {
            bail!("non-directory component in requested path");
        }
        let canonical = current.canonicalize()?;
        if !canonical.starts_with(root) {
            bail!("requested path escapes authorized root");
        }
    }
    let metadata = fs::symlink_metadata(&current)?;
    if metadata.file_type().is_symlink()
        || (!metadata.file_type().is_file() && !metadata.file_type().is_dir())
    {
        bail!("linked or special paths are not allowed");
    }
    if !current.canonicalize()?.starts_with(root) {
        bail!("requested path escapes authorized root");
    }
    Ok((current, metadata))
}

pub(super) fn selected_root(roots: &AllowedRoots, id: u32) -> Result<&Path> {
    if id == 0 {
        bail!("invalid authorized root identifier");
    }
    roots
        .roots
        .get((id - 1) as usize)
        .map(PathBuf::as_path)
        .context("unknown authorized root identifier")
}

pub(super) fn authorized_path(
    roots: &AllowedRoots,
    budget: &Budget,
    root_id: u32,
    relative: &str,
    empty: EmptyPath,
) -> Result<(PathBuf, fs::Metadata)> {
    budget.operation()?;
    let root = selected_root(roots, root_id)?;
    if roots.hides(root_id, relative) {
        bail!("cannot inspect requested path {relative:?}: the update replaces it");
    }
    safe_target(root, relative, empty)
}
