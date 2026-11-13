//! Digest checks for the bytes that an undo would restore.
use crate::archive::{ArchiveFormat, Inspection};
use crate::planning::InstallPlan;
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

/// SHA-256 of every archive member that `plan` selects, keyed by root and
/// destination.
pub(super) fn selected_digests(
    archive: &Path,
    plan: &InstallPlan,
    staging_root: &Path,
) -> Result<BTreeMap<(String, String), String>> {
    let inspection = crate::archive::inspect(archive)?;
    match inspection.format {
        ArchiveFormat::Zip => zip_digests(archive, &inspection, plan),
        ArchiveFormat::SevenZ => sevenz_digests(archive, &inspection, plan, staging_root),
        _ => bail!("archive format cannot be installed"),
    }
}

fn zip_digests(
    archive: &Path,
    inspection: &Inspection,
    plan: &InstallPlan,
) -> Result<BTreeMap<(String, String), String>> {
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
        let name = original
            .get(operation.source.as_str())
            .copied()
            .context("source is not present in inspected ZIP")?;
        let mut input = zip.by_name(name)?;
        if input.is_dir() {
            bail!("archive metadata changed for {}", operation.source);
        }
        let mut hasher = Sha256::new();
        let copied = io::copy(&mut input.by_ref().take(operation.bytes + 1), &mut hasher)?;
        if copied != operation.bytes {
            bail!("archive extraction size mismatch for {}", operation.source);
        }
        digests.insert(
            (operation.root.clone(), operation.destination.clone()),
            format!("{:x}", hasher.finalize()),
        );
    }
    Ok(digests)
}

fn sevenz_digests(
    archive: &Path,
    inspection: &Inspection,
    plan: &InstallPlan,
    staging_root: &Path,
) -> Result<BTreeMap<(String, String), String>> {
    let staging = tempfile::Builder::new()
        .prefix("undo-check-")
        .tempdir_in(staging_root)?;
    let control = crate::progress::OperationControl::default();
    let files = super::staging::stage_7z(archive, inspection, plan, staging.path(), &control)?;
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
