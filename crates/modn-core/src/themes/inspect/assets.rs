//! Asset scanning for one theme directory.

use super::super::limits::{is_valid_asset_path, MAX_THEME_ASSET_BYTES};
use super::super::problem::{Rule, ThemeProblem};
use super::reading::read_prefix;
use std::collections::BTreeSet;
use std::fs::{self, DirEntry};
use std::path::Path;

/// The deepest asset directory nest accepted inside one theme.
const MAX_ASSET_DEPTH: usize = 32;

/// File type of one theme asset, decided by its extension.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AssetKind {
    /// A PNG image.
    Png,
    /// A WebP image.
    Webp,
    /// A WOFF2 font.
    Woff2,
}

impl AssetKind {
    /// Return the asset kind for a lowercase extension without the dot.
    #[must_use]
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension {
            "png" => Some(Self::Png),
            "webp" => Some(Self::Webp),
            "woff2" => Some(Self::Woff2),
            _ => None,
        }
    }

    /// Return the lowercase extension, without the dot.
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Webp => "webp",
            Self::Woff2 => "woff2",
        }
    }
}

/// One regular file under `assets/`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetInfo {
    /// Path relative to the theme directory, such as `assets/icon.png`.
    pub path: String,
    /// File size in bytes.
    pub bytes: u64,
    /// Asset type confirmed by the magic bytes.
    pub kind: AssetKind,
}

impl AssetInfo {
    /// Create one asset record.
    #[must_use]
    pub fn new(path: impl Into<String>, bytes: u64, kind: AssetKind) -> Self {
        Self {
            path: path.into(),
            bytes,
            kind,
        }
    }
}

/// The result of one recursive scan of `assets/`.
#[derive(Debug, Default)]
pub(super) struct AssetScan {
    pub(super) files: Vec<AssetInfo>,
    pub(super) present: BTreeSet<String>,
    pub(super) total_bytes: u64,
    pub(super) count: usize,
    pub(super) problems: Vec<ThemeProblem>,
}

/// Walk `dir`, which is the theme's `assets/` directory.
pub(super) fn scan_assets(dir: &Path) -> AssetScan {
    let mut scan = AssetScan::default();
    let mut stack = vec![(dir.to_path_buf(), String::from("assets"), 0usize)];
    while let Some((directory, prefix, depth)) = stack.pop() {
        let entries = match read_sorted(&directory, &prefix, &mut scan.problems) {
            Some(entries) => entries,
            None => continue,
        };
        for entry in entries {
            scan_entry(&entry, &prefix, depth, &mut stack, &mut scan);
        }
    }
    scan.files.sort_by(|left, right| left.path.cmp(&right.path));
    scan
}

fn read_sorted(
    directory: &Path,
    prefix: &str,
    problems: &mut Vec<ThemeProblem>,
) -> Option<Vec<DirEntry>> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            problems.push(problem(
                prefix,
                Rule::UnexpectedFile,
                format!("cannot list \"{prefix}\": {error}"),
            ));
            return None;
        }
    };
    let mut collected = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => collected.push(entry),
            Err(error) => problems.push(problem(
                prefix,
                Rule::UnexpectedFile,
                format!("cannot list an entry in \"{prefix}\": {error}"),
            )),
        }
    }
    collected.sort_by_key(DirEntry::file_name);
    Some(collected)
}

fn scan_entry(
    entry: &DirEntry,
    prefix: &str,
    depth: usize,
    stack: &mut Vec<(std::path::PathBuf, String, usize)>,
    scan: &mut AssetScan,
) {
    let name = match entry.file_name().into_string() {
        Ok(name) => name,
        Err(name) => {
            scan.problems.push(problem(
                prefix,
                Rule::UnexpectedFile,
                format!(
                    "asset name \"{}\" is not valid UTF-8",
                    name.to_string_lossy()
                ),
            ));
            return;
        }
    };
    let relative = format!("{prefix}/{name}");
    let file_type = match entry.file_type() {
        Ok(file_type) => file_type,
        Err(error) => {
            scan.problems.push(problem(
                &relative,
                Rule::UnexpectedFile,
                format!("cannot inspect \"{relative}\": {error}"),
            ));
            return;
        }
    };
    if file_type.is_symlink() {
        scan.problems.push(problem(
            &relative,
            Rule::UnexpectedFile,
            format!("link \"{relative}\" is refused; links are not allowed"),
        ));
        return;
    }
    if file_type.is_dir() {
        if depth + 1 > MAX_ASSET_DEPTH {
            scan.problems.push(problem(
                &relative,
                Rule::UnexpectedFile,
                format!("asset directories nest deeper than {MAX_ASSET_DEPTH} levels"),
            ));
        } else {
            stack.push((entry.path(), relative, depth + 1));
        }
        return;
    }
    if !file_type.is_file() {
        scan.problems.push(problem(
            &relative,
            Rule::UnexpectedFile,
            format!("\"{relative}\" is not a regular file"),
        ));
        return;
    }
    let bytes = match entry.metadata() {
        Ok(metadata) => metadata.len(),
        Err(error) => {
            scan.problems.push(problem(
                &relative,
                Rule::UnexpectedFile,
                format!("cannot inspect \"{relative}\": {error}"),
            ));
            return;
        }
    };
    scan.count += 1;
    scan.total_bytes = scan.total_bytes.saturating_add(bytes);
    scan.present.insert(relative.clone());
    if !is_valid_asset_path(&relative) {
        scan.problems.push(problem(
            &relative,
            Rule::AssetType,
            format!("asset path \"{relative}\" is not allowed"),
        ));
        return;
    }
    let Some((_, extension)) = relative.rsplit_once('.') else {
        scan.problems.push(problem(
            &relative,
            Rule::AssetType,
            format!("asset \"{relative}\" has no .png, .webp or .woff2 extension"),
        ));
        return;
    };
    let Some(kind) = AssetKind::from_extension(extension) else {
        scan.problems.push(problem(
            &relative,
            Rule::AssetType,
            format!("asset \"{relative}\" is not a .png, .webp or .woff2 file"),
        ));
        return;
    };
    match read_prefix(&entry.path(), 12) {
        Ok(prefix) if signature_matches(kind, &prefix) => {}
        Ok(_) => {
            scan.problems.push(problem(
                &relative,
                Rule::AssetType,
                format!(
                    "asset \"{relative}\" does not start with {} bytes",
                    kind.extension()
                ),
            ));
            return;
        }
        Err(error) => {
            scan.problems.push(problem(
                &relative,
                Rule::UnexpectedFile,
                format!("cannot read \"{relative}\": {error}"),
            ));
            return;
        }
    }
    if bytes > MAX_THEME_ASSET_BYTES {
        scan.problems.push(problem(
            &relative,
            Rule::AssetTooLarge,
            format!("asset \"{relative}\" is {bytes} bytes; the limit is {MAX_THEME_ASSET_BYTES}"),
        ));
        return;
    }
    scan.files.push(AssetInfo::new(relative, bytes, kind));
}

fn signature_matches(kind: AssetKind, prefix: &[u8]) -> bool {
    match kind {
        AssetKind::Png => prefix.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
        AssetKind::Webp => {
            prefix.len() >= 12 && &prefix[..4] == b"RIFF" && &prefix[8..12] == b"WEBP"
        }
        AssetKind::Woff2 => prefix.starts_with(b"wOF2"),
    }
}

fn problem(path: &str, rule: Rule, message: impl Into<String>) -> ThemeProblem {
    ThemeProblem::new(rule, message).with_path(path)
}
