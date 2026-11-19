//! BLAKE3 digest over one theme directory.

use super::problem::{Rule, ThemeProblem};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

/// The deepest directory nest accepted while hashing a theme.
const MAX_THEME_DEPTH: usize = 32;

/// Compute the BLAKE3 digest of every regular file under `dir`.
///
/// The digest is identical on every platform for the same tree. Files are
/// ordered byte-wise by their relative path with `/` separators, and each one
/// contributes its path bytes, one NUL byte, its length as 8 little-endian
/// bytes, then its content. The result is 64 lowercase hex characters.
///
/// Links anywhere in the tree are refused with [`Rule::UnexpectedFile`].
pub fn theme_digest(dir: &Path) -> Result<String, ThemeProblem> {
    let metadata = fs::symlink_metadata(dir).map_err(|error| {
        problem(
            ".",
            Rule::ThemeMissing,
            format!("cannot inspect the theme directory: {error}"),
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(problem(
            ".",
            Rule::UnexpectedFile,
            "the theme directory is a link; links are refused",
        ));
    }
    if !metadata.is_dir() {
        return Err(problem(
            ".",
            Rule::ThemeMissing,
            "the theme path is not a directory",
        ));
    }
    let mut files = Vec::new();
    collect(dir, "", 0, &mut files)?;
    files.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    let mut hasher = blake3::Hasher::new();
    for (relative, absolute) in files {
        hash_file(&mut hasher, &relative, &absolute)?;
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn collect(
    directory: &Path,
    prefix: &str,
    depth: usize,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), ThemeProblem> {
    if depth > MAX_THEME_DEPTH {
        return Err(problem(
            or_dot(prefix),
            Rule::UnexpectedFile,
            format!("theme directory nests deeper than {MAX_THEME_DEPTH} levels"),
        ));
    }
    let entries = fs::read_dir(directory)
        .map_err(|error| io_problem(or_dot(prefix), format!("cannot list directory: {error}")))?;
    for entry in entries {
        let entry = entry
            .map_err(|error| io_problem(or_dot(prefix), format!("cannot list entry: {error}")))?;
        let name = match entry.file_name().into_string() {
            Ok(name) => name,
            Err(name) => {
                return Err(problem(
                    or_dot(prefix),
                    Rule::UnexpectedFile,
                    format!(
                        "entry \"{}\" has a name that is not valid UTF-8",
                        name.to_string_lossy()
                    ),
                ));
            }
        };
        let relative = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let file_type = entry
            .file_type()
            .map_err(|error| io_problem(&relative, format!("cannot inspect entry: {error}")))?;
        if file_type.is_symlink() {
            return Err(problem(
                &relative,
                Rule::UnexpectedFile,
                format!("link \"{relative}\" is refused"),
            ));
        }
        if file_type.is_dir() {
            collect(&entry.path(), &relative, depth + 1, files)?;
        } else if file_type.is_file() {
            files.push((relative, entry.path()));
        } else {
            return Err(problem(
                &relative,
                Rule::UnexpectedFile,
                format!("\"{relative}\" is not a regular file"),
            ));
        }
    }
    Ok(())
}

fn hash_file(hasher: &mut blake3::Hasher, relative: &str, path: &Path) -> Result<(), ThemeProblem> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| io_problem(relative, format!("cannot inspect file: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(problem(
            relative,
            Rule::UnexpectedFile,
            format!("\"{relative}\" changed while the digest was computed"),
        ));
    }
    hasher.update(relative.as_bytes());
    hasher.update(&[0]);
    hasher.update(&metadata.len().to_le_bytes());
    let mut file = File::open(path)
        .map_err(|error| io_problem(relative, format!("cannot open file: {error}")))?;
    let mut buffer = [0u8; 64 * 1024];
    let mut read_total = 0u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| io_problem(relative, format!("cannot read file: {error}")))?;
        if read == 0 {
            break;
        }
        read_total = read_total.saturating_add(read as u64);
        hasher.update(&buffer[..read]);
    }
    if read_total != metadata.len() {
        return Err(problem(
            relative,
            Rule::UnexpectedFile,
            format!("\"{relative}\" changed while the digest was computed"),
        ));
    }
    Ok(())
}

fn or_dot(path: &str) -> &str {
    if path.is_empty() {
        "."
    } else {
        path
    }
}

fn problem(path: &str, rule: Rule, message: impl Into<String>) -> ThemeProblem {
    ThemeProblem::new(rule, message).with_path(path)
}

fn io_problem(path: &str, message: impl Into<String>) -> ThemeProblem {
    ThemeProblem::new(Rule::UnexpectedFile, message).with_path(path)
}

#[cfg(test)]
#[path = "digest_tests.rs"]
mod tests;
