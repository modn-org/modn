//! Stream validated 7z members into the same private transaction staging area as ZIP.
use super::journal::{digest, JournalFile};
use crate::archive::Inspection;
use crate::planning::InstallPlan;
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::path::Path;

/// Decode solid and non-solid 7z archives once, without buffering file payloads
/// in memory or extracting any unselected entries to the filesystem.
pub(super) fn stage_7z(
    archive: &Path,
    inspection: &Inspection,
    plan: &InstallPlan,
    staging: &Path,
) -> Result<Vec<JournalFile>> {
    let original: HashMap<_, _> = inspection
        .files
        .iter()
        .map(|entry| {
            (
                entry.entry.as_str(),
                entry.original_entry.as_deref().unwrap_or(&entry.entry),
            )
        })
        .collect();
    let selected: HashMap<_, _> = plan
        .operations
        .iter()
        .enumerate()
        .map(|(ordinal, op)| (original[op.source.as_str()], ordinal))
        .collect();
    let mut staged: Vec<Option<JournalFile>> = std::iter::repeat_with(|| None)
        .take(plan.operations.len())
        .collect();
    let mut reader = sevenz_rust2::ArchiveReader::open(archive, sevenz_rust2::Password::empty())?;
    reader.for_each_entries(|entry, input| {
        if entry.is_directory {
            return Ok(true);
        }
        let Some(&ordinal) = selected.get(entry.name.as_str()) else {
            // Solid 7z blocks are sequential: even ignored files must be drained
            // before the decoder can reach the next selected member.
            let copied = io::copy(&mut input.take(entry.size + 1), &mut io::sink())?;
            if copied != entry.size {
                return Err(sevenz_rust2::Error::Other(
                    format!("truncated unselected 7z entry: {}", entry.name).into(),
                ));
            }
            return Ok(true);
        };
        let op = &plan.operations[ordinal];
        if staged[ordinal].is_some() || entry.size != op.bytes {
            return Err(sevenz_rust2::Error::Other(
                format!("duplicate or changed archive entry: {}", entry.name).into(),
            ));
        }
        let copy = (|| -> io::Result<JournalFile> {
            let stage = staging.join(format!("{ordinal}.bin"));
            let mut output = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&stage)?;
            let copied = io::copy(&mut input.take(op.bytes + 1), &mut output)?;
            output.flush()?;
            output.sync_all()?;
            if copied != op.bytes {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("7z extraction size mismatch: {}", op.source),
                ));
            }
            let sha256 = digest(&stage).map_err(io::Error::other)?;
            Ok(JournalFile {
                ordinal,
                destination: op.destination.clone(),
                sha256,
                bytes: copied,
            })
        })()
        .map_err(sevenz_rust2::Error::from)?;
        staged[ordinal] = Some(copy);
        Ok(true)
    })?;
    staged
        .into_iter()
        .enumerate()
        .map(|(ordinal, file)| {
            file.ok_or_else(|| {
                anyhow!(
                    "missing selected 7z member: {}",
                    plan.operations[ordinal].source
                )
            })
        })
        .collect()
}
