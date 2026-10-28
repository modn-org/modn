//! Header-only inspection; never extracts an archive or executes contained code.
use anyhow::{bail, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};

const MAX_ENTRIES: usize = 20_000;
const MAX_UNPACKED: u64 = 64 * 1024 * 1024 * 1024;
const MAX_ARCHIVE: u64 = 16 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct ArchiveFile {
    pub entry: String,
    pub bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct Inspection {
    pub archive: PathBuf,
    pub format: String,
    pub sha256: String,
    pub total_uncompressed_bytes: u64,
    pub files: Vec<ArchiveFile>,
    pub metadata_only: bool,
}

fn validated_name(name: &str) -> Result<String> {
    if name.is_empty() || name.contains('\\') || name.starts_with('/') || name.contains('\0') {
        bail!("unsafe archive entry: {name:?}");
    }
    let mut segments = Vec::new();
    for component in name.trim_end_matches('/').split('/') {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.contains(':')
            || component.ends_with([' ', '.'])
            || component.chars().any(|ch| ch.is_control())
        {
            bail!("unsafe archive entry: {name:?}");
        }
        let stem = component
            .split('.')
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit()
                && stem.as_bytes()[3] != b'0')
        {
            bail!("Windows reserved archive path: {name:?}");
        }
        segments.push(component);
    }
    if segments.is_empty() {
        bail!("empty archive entry");
    }
    Ok(segments.join("/"))
}

fn append_file(
    files: &mut Vec<ArchiveFile>,
    seen: &mut HashSet<String>,
    name: &str,
    size: u64,
    total: &mut u64,
) -> Result<()> {
    let name = validated_name(name)?;
    let key = name.to_lowercase();
    if !seen.insert(key) {
        bail!("duplicate or case-colliding archive entry: {name}");
    }
    *total = total.checked_add(size).context("archive size overflow")?;
    if *total > MAX_UNPACKED {
        bail!("archive exceeds 64 GiB uncompressed metadata limit");
    }
    files.push(ArchiveFile {
        entry: name,
        bytes: size,
    });
    Ok(())
}

pub fn inspect(path: &Path) -> Result<Inspection> {
    let metadata = path
        .metadata()
        .with_context(|| format!("cannot access {}", path.display()))?;
    if !metadata.is_file() || metadata.len() > MAX_ARCHIVE {
        bail!("archive must be a file no larger than 16 GiB");
    }
    let format = match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "zip" => "zip",
        "jar" => "jar",
        "7z" => "7z",
        _ => bail!("unsupported archive format (expected .zip, .jar, or .7z)"),
    };
    let mut hasher = Sha256::new();
    std::io::copy(&mut File::open(path)?, &mut hasher)?;
    let sha256 = format!("{:x}", hasher.finalize());

    let mut files = Vec::new();
    let mut seen = HashSet::new();
    let mut total = 0u64;
    if format == "7z" {
        let reader = sevenz_rust2::ArchiveReader::open(path, sevenz_rust2::Password::empty())
            .context("invalid or encrypted 7z archive")?;
        if reader.archive().files.len() > MAX_ENTRIES {
            bail!("archive has more than {MAX_ENTRIES} entries");
        }
        for entry in &reader.archive().files {
            validated_name(&entry.name)?;
            if entry.is_anti_item
                || (entry.has_windows_attributes
                    && (entry.windows_attributes & 0x400 != 0
                        || ((entry.windows_attributes >> 16) & 0o170000) == 0o120000))
            {
                bail!("link or deletion entry is not supported: {}", entry.name);
            }
            if !entry.is_directory {
                append_file(&mut files, &mut seen, &entry.name, entry.size, &mut total)?;
            }
        }
    } else {
        let mut reader = zip::ZipArchive::new(BufReader::new(File::open(path)?))
            .context("invalid ZIP/JAR archive")?;
        if reader.len() > MAX_ENTRIES {
            bail!("archive has more than {MAX_ENTRIES} entries");
        }
        for i in 0..reader.len() {
            let entry = reader
                .by_index_raw(i)
                .context("cannot read ZIP entry metadata")?;
            validated_name(entry.name())?;
            if entry.encrypted()
                || entry
                    .unix_mode()
                    .is_some_and(|mode| mode & 0o170000 == 0o120000)
            {
                bail!(
                    "encrypted or symlink entry is not supported: {}",
                    entry.name()
                );
            }
            if !entry.is_dir() {
                append_file(
                    &mut files,
                    &mut seen,
                    entry.name(),
                    entry.size(),
                    &mut total,
                )?;
            }
        }
    }
    if files.is_empty() {
        bail!("archive has no files");
    }
    files.sort_by(|a, b| a.entry.cmp(&b.entry));
    Ok(Inspection {
        archive: path.to_path_buf(),
        format: format.into(),
        sha256,
        total_uncompressed_bytes: total,
        files,
        metadata_only: true,
    })
}
