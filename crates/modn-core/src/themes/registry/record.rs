//! The registration record stored beside one copied theme.

use super::super::limits::is_valid_id;
use super::layout;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

/// Largest accepted `registration.json`, which is far above its real size.
const MAX_RECORD_BYTES: u64 = 4096;

/// Contents of one `registration.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct RegistrationRecord {
    /// Shape of this record; must be [`layout::SCHEMA_VERSION`].
    pub(super) schema_version: u32,
    /// Game the theme belongs to.
    pub(super) game: String,
    /// Theme id.
    pub(super) id: String,
    /// BLAKE3 digest of the copied theme directory.
    pub(super) blake3: String,
    /// Unix seconds when the theme was registered.
    pub(super) registered_at: u64,
}

impl RegistrationRecord {
    /// Check every field that does not depend on the directory name.
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != layout::SCHEMA_VERSION {
            return Err(format!(
                "registration.json schema_version {} is not {}",
                self.schema_version,
                layout::SCHEMA_VERSION
            ));
        }
        if !is_valid_id(&self.game) {
            return Err(format!(
                "registration.json game \"{}\" is not a valid game id",
                self.game
            ));
        }
        if !is_valid_id(&self.id) {
            return Err(format!(
                "registration.json id \"{}\" is not a valid theme id",
                self.id
            ));
        }
        if self.blake3.len() != 64
            || !self
                .blake3
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err("registration.json blake3 is not 64 lowercase hex characters".to_owned());
        }
        Ok(())
    }
}

/// Read `registration.json` from one registration directory.
///
/// Every failure, including a missing or unreadable file, returns a reason the
/// caller can turn into a diagnostic. The caller checks that the recorded game
/// and id match the directory names.
pub(super) fn read(entry: &Path) -> Result<RegistrationRecord, String> {
    let path = entry.join(layout::RECORD);
    let metadata = match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err("registration.json is missing".to_owned());
        }
        Err(error) => return Err(format!("cannot inspect registration.json: {error}")),
        Ok(metadata) => metadata,
    };
    if !metadata.file_type().is_file() {
        return Err("registration.json is not a regular file".to_owned());
    }
    if metadata.len() > MAX_RECORD_BYTES {
        return Err(format!(
            "registration.json exceeds the {MAX_RECORD_BYTES}-byte limit"
        ));
    }
    let bytes =
        fs::read(&path).map_err(|error| format!("cannot read registration.json: {error}"))?;
    let record: RegistrationRecord = serde_json::from_slice(&bytes)
        .map_err(|error| format!("registration.json is not valid: {error}"))?;
    record.validate()?;
    Ok(record)
}

/// Write one `registration.json` into a staging directory.
pub(super) fn write(
    entry: &Path,
    record: &RegistrationRecord,
) -> Result<(), super::super::error::ThemeError> {
    let path = entry.join(layout::RECORD);
    let bytes = serde_json::to_vec(record).map_err(|error| {
        super::super::error::ThemeError::corrupt(
            &path,
            format!("cannot encode registration.json: {error}"),
        )
    })?;
    let mut file = File::create(&path).map_err(|error| {
        super::super::error::ThemeError::io("cannot create registration.json", error)
    })?;
    file.write_all(&bytes).map_err(|error| {
        super::super::error::ThemeError::io("cannot write registration.json", error)
    })?;
    file.sync_all().map_err(|error| {
        super::super::error::ThemeError::io("cannot sync registration.json", error)
    })
}
