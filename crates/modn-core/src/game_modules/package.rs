//! Strict, bounded local ZIP packages. Hashes identify bytes, never publishers.
use super::capabilities::{AllowedRoots, ReadProfile};
use super::loader::{self, ModuleManifest, MAX_MANIFEST_BYTES, MAX_SCRIPT_BYTES};
use super::runtime;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::Path;

const MAX_PACKAGE_BYTES: u64 = 288 * 1024;

/// Integrity and compatibility of one unsigned package; no activation implied.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct PackageInfo {
    /// Manifest and runtime game identity.
    pub game_id: String,
    /// Opaque version label; identical labels cannot be replaced.
    pub version: String,
    /// Required host API revision.
    pub api_version: u32,
    /// Flat source filename.
    pub entrypoint: String,
    /// Content address of length-prefixed manifest and source bytes.
    pub sha256: String,
    /// Digest of the exact manifest bytes.
    pub manifest_sha256: String,
    /// Digest of the exact source bytes.
    pub script_sha256: String,
}

/// Validated in-memory artifact. Reuse this snapshot for approval and installation.
#[derive(Debug)]
pub struct ModulePackage {
    pub(super) info: PackageInfo,
    pub(super) manifest: Vec<u8>,
    pub(super) script: Vec<u8>,
}

pub(super) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

impl ModulePackage {
    /// Return validated package identity and integrity digests.
    #[must_use]
    pub fn info(&self) -> &PackageInfo {
        &self.info
    }

    /// Apply the existing state-location exclusion check using the incoming policy.
    /// This grants only the candidate ancestor being checked, as game registration does;
    /// no root authorization is retained and the module still has no write capabilities.
    pub fn ensure_outside_game(&self, state_dir: &Path) -> Result<()> {
        let source = std::str::from_utf8(&self.script)?;
        let existing = state_dir
            .ancestors()
            .find(|path| path.exists())
            .context("cannot resolve module state directory")?
            .canonicalize()?;
        for candidate in existing.ancestors() {
            // Match the existing manager guard: inaccessible ancestors cannot be
            // recognized as games and must not block an otherwise writable store.
            let Ok(roots) = AllowedRoots::validate(&[candidate.to_path_buf()]) else {
                continue;
            };
            let recognized = runtime::evaluate(
                source,
                &self.info.entrypoint,
                roots,
                ReadProfile::Standard,
                |_, output| {
                    let id: String = output.get("game_id")?;
                    let api: u32 = output.get("api_version")?;
                    if id != self.info.game_id || api != self.info.api_version {
                        return Err(mlua::Error::RuntimeError(
                            "module identity mismatch during state-location validation".into(),
                        ));
                    }
                    match output.get::<Option<mlua::Function>>("validate")? {
                        Some(validate) => validate.call::<bool>(()),
                        None => Ok(false),
                    }
                },
            );
            if recognized.unwrap_or(false) {
                bail!("module state directory must be outside a game installation");
            }
        }
        Ok(())
    }
    pub(super) fn from_bytes(manifest: Vec<u8>, script: Vec<u8>) -> Result<Self> {
        let module = loader::check_bytes(&manifest, &script)?;
        let mut hash = Sha256::new();
        hash.update(b"modn-module-package-v1\0");
        hash.update((manifest.len() as u64).to_le_bytes());
        hash.update(&manifest);
        hash.update(&script);
        Ok(Self {
            info: PackageInfo {
                game_id: module.game_id,
                version: module.version,
                api_version: module.api_version,
                entrypoint: module.entrypoint,
                sha256: format!("{:x}", hash.finalize()),
                manifest_sha256: digest(&manifest),
                script_sha256: digest(&script),
            },
            manifest,
            script,
        })
    }

    /// Read and validate one bounded local ZIP without creating state or granting roots.
    pub fn read(path: &Path) -> Result<Self> {
        let file = File::open(path).context("cannot open local module ZIP")?;
        if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_PACKAGE_BYTES {
            bail!("module ZIP input must be a regular file no larger than 288 KiB");
        }
        let mut bytes = Vec::new();
        file.take(MAX_PACKAGE_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_PACKAGE_BYTES {
            bail!("module ZIP input exceeds 288 KiB");
        }
        validate_headers(&bytes)?;
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes))?;
        let manifest = read_entry(&mut zip, "module.json", MAX_MANIFEST_BYTES)?;
        let metadata: ModuleManifest = serde_json::from_slice(&manifest)?;
        loader::validate_manifest(&metadata)?;
        let script = read_entry(&mut zip, &metadata.entrypoint, MAX_SCRIPT_BYTES)?;
        Self::from_bytes(manifest, script)
    }
}

/// Fully inspect a local package in memory without creating state or granting roots.
pub fn inspect_package(path: &Path) -> Result<PackageInfo> {
    Ok(ModulePackage::read(path)?.info)
}

fn read_entry(
    zip: &mut zip::ZipArchive<Cursor<Vec<u8>>>,
    name: &str,
    limit: u64,
) -> Result<Vec<u8>> {
    let mut entry = zip
        .by_name(name)
        .with_context(|| format!("missing package entry {name}"))?;
    if entry.size() > limit {
        bail!("module entry {name} exceeds {limit} bytes");
    }
    let mut bytes = Vec::new();
    (&mut entry).take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 != entry.size() || bytes.len() as u64 > limit {
        bail!("module entry size mismatch");
    }
    Ok(bytes)
}

fn slice(bytes: &[u8], start: usize, length: usize) -> Result<&[u8]> {
    bytes
        .get(start..start.checked_add(length).context("ZIP offset overflow")?)
        .context("truncated module ZIP")
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(slice(bytes, offset, 2)?.try_into()?))
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(slice(bytes, offset, 4)?.try_into()?))
}

// Bound and inspect raw records before ZipArchive can allocate or deduplicate names.
fn validate_headers(bytes: &[u8]) -> Result<()> {
    let end = bytes
        .len()
        .checked_sub(22)
        .context("missing ZIP end record")?;
    if slice(bytes, end, 4)? != b"PK\x05\x06"
        || u16_at(bytes, end + 4)? != 0
        || u16_at(bytes, end + 6)? != 0
        || u16_at(bytes, end + 8)? != 2
        || u16_at(bytes, end + 10)? != 2
        || u16_at(bytes, end + 20)? != 0
    {
        bail!("module ZIP requires exactly two entries, no comments, ZIP64 or split disks");
    }
    let central_size = u32_at(bytes, end + 12)? as usize;
    let central = u32_at(bytes, end + 16)? as usize;
    if central_size > 8192 || central.checked_add(central_size) != Some(end) {
        bail!("module ZIP metadata exceeds limit or has invalid offsets");
    }
    let mut position = central;
    let mut local = 0;
    let mut seen = HashSet::new();
    for _ in 0..2 {
        let header = slice(bytes, position, 46)?;
        if &header[..4] != b"PK\x01\x02" {
            bail!("invalid module ZIP central record");
        }
        let flags = u16_at(header, 8)?;
        let method = u16_at(header, 10)?;
        let name_len = u16_at(header, 28)? as usize;
        let name = std::str::from_utf8(slice(bytes, position + 46, name_len)?)?;
        let attrs = u32_at(header, 38)?;
        let mode = attrs >> 16;
        if u16_at(header, 6)? > 20
            || flags & !0x800 != 0
            || !matches!(method, 0 | 8)
            || u16_at(header, 30)? != 0
            || u16_at(header, 32)? != 0
            || u16_at(header, 34)? != 0
            || attrs & 0x410 != 0
            || !matches!(mode & 0o170000, 0 | 0o100000)
            || mode & 0o111 != 0
        {
            bail!("module ZIP forbids encryption, links, executable files, extra metadata and data descriptors");
        }
        if name_len > 96
            || !(name == "module.json" || loader::valid_entrypoint(name))
            || !seen.insert(name.to_ascii_lowercase())
        {
            bail!("unsafe, extra, duplicate or case-colliding module ZIP entry: {name:?}");
        }
        let unpacked = u32_at(header, 24)? as u64;
        let limit = if name == "module.json" {
            MAX_MANIFEST_BYTES
        } else {
            MAX_SCRIPT_BYTES
        };
        if unpacked > limit || u32_at(header, 42)? as usize != local {
            bail!("module ZIP entry exceeds size limit or overlaps another entry");
        }
        let local_header = slice(bytes, local, 30)?;
        if &local_header[..4] != b"PK\x03\x04"
            || slice(local_header, 4, 22)? != slice(header, 6, 22)?
            || u16_at(local_header, 26)? as usize != name_len
            || u16_at(local_header, 28)? != 0
            || slice(bytes, local + 30, name_len)? != name.as_bytes()
        {
            bail!("module ZIP local and central headers disagree");
        }
        local = local
            .checked_add(30 + name_len)
            .and_then(|n| n.checked_add(u32_at(header, 20).ok()? as usize))
            .context("module ZIP entry offset overflow")?;
        if local > central {
            bail!("module ZIP entry overlaps metadata");
        }
        position += 46 + name_len;
    }
    if position != end || local != central || !seen.contains("module.json") {
        bail!("module ZIP has trailing, prefixed or missing entries");
    }
    Ok(())
}
