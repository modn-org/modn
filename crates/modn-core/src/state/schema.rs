use super::roots;
use anyhow::Result;
use rusqlite::Connection;
use std::path::Path;

/// Refuse a state database this build did not write, before it is read.
pub(super) fn validate_package_schema(connection: &Connection, path: &Path) -> Result<()> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version != required_schema_version() {
        return Err(schema_refusal(path, version, ""));
    }
    let (archive_required, manual_origin): (Option<i64>, bool) = connection.query_row(
        r#"SELECT
            (SELECT "notnull" FROM pragma_table_info('managed_packages')
             WHERE name='archive_blake3'),
            EXISTS(SELECT 1 FROM pragma_table_info('managed_packages') WHERE name='origin')"#,
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if archive_required != Some(1) || manual_origin {
        return Err(schema_refusal(
            path,
            version,
            "It does not use the table layout this build writes.",
        ));
    }
    Ok(())
}

fn required_schema_version() -> i64 {
    roots::SCHEMA_VERSION.max(crate::approval::SCHEMA_VERSION)
}

fn schema_refusal(path: &Path, found: i64, detail: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "state database {} has schema version {found}, but this Modn build needs schema version {}. {detail} Modn did not read or change the database. Move it aside to start a new state; files Modn installed stay on disk but are no longer tracked.",
        path.display(),
        required_schema_version()
    )
}
