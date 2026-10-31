//! Stream validated 7z members into the same private transaction staging area as ZIP.
use super::journal::{digest, JournalFile};
use crate::archive::Inspection;
use crate::planning::InstallPlan;
use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

/// The locked sevenz decoder walks each solid block using its stream count,
/// but parsed 7z headers can interleave streamless directories/empty files.
/// Group the data-bearing entries in compressed-stream order so that decoder
/// iteration cannot silently omit members after a streamless entry.
fn decoder_order(mut archive: sevenz_rust2::Archive) -> Result<sevenz_rust2::Archive> {
    let mut groups = vec![Vec::new(); archive.blocks.len()];
    let mut streamless = Vec::new();
    let original_map = std::mem::take(&mut archive.stream_map.file_block_index);
    if archive.files.len() != original_map.len() {
        return Err(anyhow!("invalid 7z stream map length"));
    }
    for (entry, block) in std::mem::take(&mut archive.files)
        .into_iter()
        .zip(original_map)
    {
        if entry.has_stream {
            let group = block
                .and_then(|id| groups.get_mut(id))
                .context("7z data stream has no valid compression block")?;
            group.push(entry);
        } else {
            streamless.push(entry);
        }
    }
    let mut map = Vec::new();
    let mut first = Vec::with_capacity(groups.len());
    for (block, group) in groups.into_iter().enumerate() {
        first.push(archive.files.len());
        for entry in group {
            archive.files.push(entry);
            map.push(Some(block));
        }
    }
    for entry in streamless {
        archive.files.push(entry);
        map.push(None);
    }
    archive.stream_map.file_block_index = map;
    archive.stream_map.block_first_file_index = first;
    Ok(archive)
}

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
    let metadata = sevenz_rust2::ArchiveReader::open(archive, sevenz_rust2::Password::empty())?
        .archive()
        .clone();
    let mut reader = sevenz_rust2::ArchiveReader::from_archive(
        decoder_order(metadata)?,
        File::open(archive)?,
        sevenz_rust2::Password::empty(),
    );
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

#[cfg(test)]
mod tests {
    use super::decoder_order;
    use sevenz_rust2::{ArchiveEntry, ArchiveReader, ArchiveWriter, Password, SourceReader};
    use std::collections::HashMap;
    use std::fs::File;
    use std::io::Cursor;
    use tempfile::tempdir;

    #[test]
    fn interleaved_streamless_directory_keeps_both_solid_members_decodable() {
        let temp = tempdir().expect("fixture");
        let path = temp.path().join("solid.7z");
        let a = "r6/scripts/Example/A.reds";
        let b = "r6/scripts/Example/B.reds";
        let directory = "r6/scripts/Example/empty/";
        let mut writer = ArchiveWriter::create(&path).expect("writer");
        writer
            .push_archive_entries(
                vec![ArchiveEntry::new_file(a), ArchiveEntry::new_file(b)],
                vec![
                    SourceReader::new(Cursor::new(b"alpha".as_slice())),
                    SourceReader::new(Cursor::new(b"bravo".as_slice())),
                ],
            )
            .expect("solid block");
        writer.finish().expect("complete archive");
        let mut parsed = ArchiveReader::open(&path, Password::empty())
            .expect("valid 7z header")
            .archive()
            .clone();
        assert!(parsed.is_solid, "fixture must use solid compression");
        // Model a valid header with a streamless directory between two data
        // streams in the same solid block. The locked library's decoder would
        // visit only A and the directory, silently missing B in this ordering.
        parsed
            .files
            .insert(1, ArchiveEntry::new_directory(directory));
        parsed.stream_map.file_block_index.insert(1, Some(0));
        let fixed = decoder_order(parsed).expect("reorder stream map");
        assert_eq!(
            fixed
                .files
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec![a, b, directory]
        );
        assert_eq!(
            fixed.stream_map.file_block_index,
            vec![Some(0), Some(0), None]
        );
        let mut decoder = ArchiveReader::from_archive(
            fixed,
            File::open(&path).expect("archive"),
            Password::empty(),
        );
        let mut actual = HashMap::new();
        decoder
            .for_each_entries(|entry, input| {
                let mut bytes = Vec::new();
                input.read_to_end(&mut bytes)?;
                actual.insert(entry.name.clone(), bytes);
                Ok(true)
            })
            .expect("decode both solid streams");
        assert_eq!(actual[a], b"alpha");
        assert_eq!(actual[b], b"bravo");
        assert_eq!(actual[directory], b"");
    }
}
