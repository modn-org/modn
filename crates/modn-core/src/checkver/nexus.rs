//! Nexus Mods file-list parsing and selection.

use super::glob::glob_matches;
use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Deserialize)]
struct FileEntry {
    file_id: u64,
    file_name: String,
    #[serde(default)]
    uploaded_timestamp: u64,
}

/// The newest Nexus file whose name matches the pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SelectedNexusFile {
    /// The numeric Nexus file ID.
    pub file_id: u64,
    /// The matching file name.
    pub file_name: String,
}

/// Select the newest file whose name matches `file_pattern`.
///
/// Upload time is the Nexus `uploaded_timestamp`. Equal timestamps choose the
/// larger file ID, so a check is deterministic.
///
/// # Errors
///
/// Returns an error for malformed JSON or when no file matches.
pub fn select_nexus_file(bytes: &[u8], file_pattern: &str) -> Result<SelectedNexusFile> {
    let files: Vec<FileEntry> = serde_json::from_slice(bytes).context("invalid Nexus file list")?;
    let mut newest: Option<&FileEntry> = None;
    for file in &files {
        if !glob_matches(file_pattern, &file.file_name) {
            continue;
        }
        let newer = newest.is_none_or(|current| {
            file.uploaded_timestamp > current.uploaded_timestamp
                || (file.uploaded_timestamp == current.uploaded_timestamp
                    && file.file_id > current.file_id)
        });
        if newer {
            newest = Some(file);
        }
    }
    let file = newest.context(format!("no Nexus file matches {file_pattern}"))?;
    Ok(SelectedNexusFile {
        file_id: file.file_id,
        file_name: file.file_name.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::select_nexus_file;

    fn files(value: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&value).expect("fixture JSON")
    }

    #[test]
    fn checkver_nexus_selects_the_newest_matching_file() {
        let bytes = files(serde_json::json!([
            {"file_id": 10, "file_name": "Mod-1.0.7z", "uploaded_timestamp": 100},
            {"file_id": 20, "file_name": "Mod-2.0.7z", "uploaded_timestamp": 200},
            {"file_id": 30, "file_name": "Other-3.0.7z", "uploaded_timestamp": 300}
        ]));
        let selected = select_nexus_file(&bytes, "Mod-*.7z").expect("file");
        assert_eq!(selected.file_id, 20);
        assert_eq!(selected.file_name, "Mod-2.0.7z");
    }

    #[test]
    fn checkver_nexus_breaks_a_timestamp_tie_by_file_id() {
        let bytes = files(serde_json::json!([
            {"file_id": 10, "file_name": "Mod-a.7z", "uploaded_timestamp": 100},
            {"file_id": 11, "file_name": "Mod-b.7z", "uploaded_timestamp": 100}
        ]));
        assert_eq!(
            select_nexus_file(&bytes, "Mod-?.7z").expect("file").file_id,
            11
        );
    }

    #[test]
    fn checkver_nexus_refuses_a_list_without_a_match() {
        let bytes = files(serde_json::json!([
            {"file_id": 10, "file_name": "other.zip", "uploaded_timestamp": 100}
        ]));
        let error = format!("{:#}", select_nexus_file(&bytes, "Mod-*.7z").unwrap_err());
        assert!(error.contains("no Nexus file"), "{error}");
    }
}
