//! Header-only inspection; never extracts an archive or executes contained code.
pub mod assets;

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::collections::HashSet;
use std::fmt;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const MAX_ENTRIES: usize = 20_000;
const MAX_UNPACKED: u64 = 64 * 1024 * 1024 * 1024;
const MAX_ARCHIVE: u64 = 16 * 1024 * 1024 * 1024;

/// Sanitized archive entry and declared size.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct ArchiveFile {
    /// Normalized archive-relative entry path.
    pub entry: String,
    /// Original entry spelling, when ZIP separators required normalization.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_entry: Option<String>,
    /// Declared uncompressed size in bytes.
    pub bytes: u64,
}

/// Container format recognized from an archive's file extension.
#[expect(missing_docs, reason = "Field and variant names state their meaning")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub enum ArchiveFormat {
    #[serde(rename = "zip")]
    Zip,
    /// `.jar` archive; inspectable but not installable.
    #[serde(rename = "jar")]
    Jar,
    #[serde(rename = "7z")]
    SevenZ,
}

impl ArchiveFormat {
    /// Serialized name, also passed to Lua modules.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::Jar => "jar",
            Self::SevenZ => "7z",
        }
    }

    /// Whether the transaction executor can extract this format.
    #[must_use]
    pub fn is_installable(self) -> bool {
        matches!(self, Self::Zip | Self::SevenZ)
    }
}

impl fmt::Display for ArchiveFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Read-only archive metadata, not verified contents or compatibility.
#[expect(missing_docs, reason = "Field and variant names state their meaning")]
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct Inspection {
    pub archive: PathBuf,
    pub format: ArchiveFormat,
    pub blake3: String,
    /// Sum of declared uncompressed sizes.
    pub total_uncompressed_bytes: u64,
    pub files: Vec<ArchiveFile>,
    /// True: no game files are extracted.
    pub metadata_only: bool,
}

pub(crate) fn validated_name(name: &str) -> Result<String> {
    if name.is_empty() || name.contains('\\') || name.starts_with('/') || name.contains('\0') {
        bail!("unsafe archive entry: {name:?}");
    }
    let mut segments = Vec::new();
    for component in name.trim_end_matches('/').split('/') {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.contains(':')
            || component
                .chars()
                .any(|ch| matches!(ch, '?' | '*' | '<' | '>' | '"' | '|'))
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
            || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.chars().count() == 4
                && stem.chars().last().is_some_and(|digit| {
                    matches!(digit, '1'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}')
                }))
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
    let normalized = name.replace('\\', "/");
    let normalized = validated_name(&normalized)?;
    let original_entry = (normalized != name).then(|| name.to_owned());
    let name = normalized;
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
        original_entry,
        bytes: size,
    });
    Ok(())
}

// ZipArchive deduplicates identical names; inspect raw central records first.
fn verify_zip_central_records(path: &Path, start: u64) -> Result<()> {
    let mut input = File::open(path)?;
    input.seek(SeekFrom::Start(start))?;
    let mut seen = HashSet::new();
    let mut count = 0;
    loop {
        let mut signature = [0u8; 4];
        input
            .read_exact(&mut signature)
            .context("truncated ZIP central directory")?;
        if signature != [0x50, 0x4b, 0x01, 0x02] {
            break;
        }
        count += 1;
        if count > MAX_ENTRIES {
            bail!("archive has more than {MAX_ENTRIES} entries");
        }
        let mut header = [0u8; 42];
        input
            .read_exact(&mut header)
            .context("truncated ZIP central record")?;
        let name_len = u16::from_le_bytes([header[24], header[25]]) as usize;
        let extra_len = u16::from_le_bytes([header[26], header[27]]) as i64;
        let comment_len = u16::from_le_bytes([header[28], header[29]]) as i64;
        let mut name = vec![0; name_len];
        input
            .read_exact(&mut name)
            .context("truncated ZIP central name")?;
        if !seen.insert(name.clone()) {
            bail!(
                "duplicate ZIP central record: {}",
                String::from_utf8_lossy(&name)
            );
        }
        input.seek(SeekFrom::Current(extra_len + comment_len))?;
    }
    if count == 0 {
        bail!("archive contains no ZIP central records");
    }
    Ok(())
}

/// Inspect validated archive headers and hash the archive without extracting game files.
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
        "zip" => ArchiveFormat::Zip,
        "jar" => ArchiveFormat::Jar,
        "7z" => ArchiveFormat::SevenZ,
        _ => bail!("unsupported archive format (expected .zip, .jar, or .7z)"),
    };
    let mut hasher = blake3::Hasher::new();
    std::io::copy(&mut File::open(path)?, &mut hasher)?;
    let blake3 = hasher.finalize().to_hex().to_string();

    let mut files = Vec::new();
    let mut seen = HashSet::new();
    let mut total = 0u64;
    if format == ArchiveFormat::SevenZ {
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
        verify_zip_central_records(path, reader.central_directory_start())?;
        if reader.len() > MAX_ENTRIES {
            bail!("archive has more than {MAX_ENTRIES} entries");
        }
        for i in 0..reader.len() {
            let entry = reader
                .by_index_raw(i)
                .context("cannot read ZIP entry metadata")?;
            validated_name(&entry.name().replace('\\', "/"))?;
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
        format,
        blake3,
        total_uncompressed_bytes: total,
        files,
        metadata_only: true,
    })
}

#[cfg(test)]
mod tests {
    use super::inspect;
    use std::fs;
    use std::io::Write;

    fn zip_fixture(path: &std::path::Path) {
        let file = fs::File::create(path).expect("archive");
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("Demo/plugin.txt", zip::write::SimpleFileOptions::default())
            .expect("entry");
        zip.write_all(b"payload").expect("payload");
        zip.finish().expect("finish");
    }

    #[test]
    fn inspection_returns_the_blake3_digest_of_the_file_bytes() {
        let dir = tempfile::tempdir().expect("directory");
        let path = dir.path().join("fixture.zip");
        zip_fixture(&path);
        let first = inspect(&path).expect("inspect");
        let second = inspect(&path).expect("inspect again");
        assert_eq!(first.blake3, second.blake3, "the digest is stable");
        assert_eq!(first.blake3.len(), 64);
        let expected = blake3::hash(&fs::read(&path).expect("bytes"))
            .to_hex()
            .to_string();
        assert_eq!(first.blake3, expected);
    }
}
