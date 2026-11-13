//! Digest checks for the bytes that an undo would restore.
use crate::archive::{ArchiveFormat, Inspection};
use crate::planning::InstallPlan;
use crate::progress::OperationControl;
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Bytes read between cancellation checks while a ZIP member is hashed.
const DIGEST_CHUNK_BYTES: usize = 64 * 1024;

/// SHA-256 of every archive member that `plan` selects, keyed by root and
/// destination.
///
/// `control` is checked before the archive is inspected and while its members
/// are read, so a cancellation before the caller's commit point stops the
/// check with [`crate::progress::Cancelled`].
pub(super) fn selected_digests(
    archive: &Path,
    plan: &InstallPlan,
    staging_root: &Path,
    control: &OperationControl,
) -> Result<BTreeMap<(String, String), String>> {
    control.checkpoint()?;
    let inspection = crate::archive::inspect(archive)?;
    match inspection.format {
        ArchiveFormat::Zip => zip_digests(archive, &inspection, plan, control),
        ArchiveFormat::SevenZ => sevenz_digests(archive, &inspection, plan, staging_root, control),
        _ => bail!("archive format cannot be installed"),
    }
}

fn zip_digests(
    archive: &Path,
    inspection: &Inspection,
    plan: &InstallPlan,
    control: &OperationControl,
) -> Result<BTreeMap<(String, String), String>> {
    control.checkpoint()?;
    let original: HashMap<&str, &str> = inspection
        .files
        .iter()
        .map(|file| {
            (
                file.entry.as_str(),
                file.original_entry.as_deref().unwrap_or(&file.entry),
            )
        })
        .collect();
    let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
    let mut digests = BTreeMap::new();
    for operation in &plan.operations {
        control.checkpoint()?;
        let name = original
            .get(operation.source.as_str())
            .copied()
            .context("source is not present in inspected ZIP")?;
        let mut input = zip.by_name(name)?;
        if input.is_dir() {
            bail!("archive metadata changed for {}", operation.source);
        }
        let digest = member_digest(&mut input, operation.bytes, control, &operation.source)?;
        digests.insert(
            (operation.root.clone(), operation.destination.clone()),
            digest,
        );
    }
    Ok(digests)
}

fn member_digest(
    input: &mut impl Read,
    bytes: u64,
    control: &OperationControl,
    source: &str,
) -> Result<String> {
    let mut hasher = Sha256::new();
    let mut remaining = bytes.saturating_add(1);
    let mut copied = 0u64;
    let mut buffer = [0u8; DIGEST_CHUNK_BYTES];
    while remaining > 0 {
        control.checkpoint()?;
        let limit = remaining.min(DIGEST_CHUNK_BYTES as u64) as usize;
        let read = input.read(&mut buffer[..limit])?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        copied += read as u64;
        remaining -= read as u64;
    }
    if copied != bytes {
        bail!("archive extraction size mismatch for {source}");
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn sevenz_digests(
    archive: &Path,
    inspection: &Inspection,
    plan: &InstallPlan,
    staging_root: &Path,
    control: &OperationControl,
) -> Result<BTreeMap<(String, String), String>> {
    control.checkpoint()?;
    let staging = tempfile::Builder::new()
        .prefix("undo-check-")
        .tempdir_in(staging_root)?;
    let files = super::staging::stage_7z(archive, inspection, plan, staging.path(), control)?;
    Ok(files
        .into_iter()
        .map(|file| {
            (
                (file.root.label().to_owned(), file.destination),
                file.sha256,
            )
        })
        .collect())
}
