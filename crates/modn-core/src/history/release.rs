//! Release metadata stored beside managed packages, and its additive schema.

use super::{HistoryContext, ReleaseMetadata};
use crate::transactions::{CatalogRelease, ReleaseDeclarations};
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

/// Release metadata for one side of an operation.
pub(crate) fn metadata_for(
    context: &HistoryContext,
    package_id: &str,
    release: Option<&CatalogRelease>,
    archive_blake3: Option<&str>,
) -> ReleaseMetadata {
    let standalone = if release.is_none() {
        context.standalone.as_ref()
    } else {
        None
    };
    ReleaseMetadata {
        mod_id: release
            .map(|release| release.reference.mod_id.clone())
            .or_else(|| standalone.map(|package| package.mod_id.clone()))
            .unwrap_or_else(|| package_id.to_owned()),
        version: release
            .and_then(|release| release.version.clone())
            .or_else(|| standalone.and_then(|package| package.version.clone())),
        source: release
            .and_then(|release| context.sources.get(&release.reference.to_string()).cloned())
            .or_else(|| standalone.map(|package| package.source.clone())),
        archive_blake3: archive_blake3.map(str::to_owned),
        catalog: release.map(|release| release.reference.clone()),
        declarations: Some(release.map_or_else(
            || ReleaseDeclarations {
                dependencies: Vec::new(),
                conflicts: Vec::new(),
                native_injector: false,
            },
            ReleaseDeclarations::of,
        )),
    }
}

/// Persist the release metadata that a later update or uninstall records.
pub(crate) fn record_metadata(
    tx: &Transaction<'_>,
    package_id: &str,
    metadata: &ReleaseMetadata,
) -> Result<()> {
    let source = metadata
        .source
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let catalog = metadata
        .catalog
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let declarations = metadata
        .declarations
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    tx.execute(
        "INSERT OR REPLACE INTO managed_package_releases
             (package_id, mod_id, version, source, archive_blake3, catalog, declarations)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            package_id,
            metadata.mod_id,
            metadata.version,
            source,
            metadata.archive_blake3,
            catalog,
            declarations
        ],
    )?;
    Ok(())
}

/// Read the release metadata of `package_id`, when recorded.
pub(crate) fn package_metadata(
    connection: &Connection,
    package_id: &str,
) -> Result<Option<ReleaseMetadata>> {
    if !has_table(connection, "managed_package_releases")? {
        return Ok(None);
    }
    let recorded = has_column(connection, "managed_package_releases", "catalog")?
        && has_column(connection, "managed_package_releases", "declarations")?;
    let (catalog, declarations) = if recorded {
        ("catalog", "declarations")
    } else {
        ("NULL", "NULL")
    };
    let mut statement = connection.prepare(&format!(
        "SELECT mod_id, version, source, archive_blake3, {catalog}, {declarations}
         FROM managed_package_releases WHERE package_id=?1"
    ))?;
    let row = statement
        .query_row([package_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .optional()?;
    row.map(
        |(mod_id, version, source, archive_blake3, catalog, declarations)| {
            Ok(ReleaseMetadata {
                mod_id,
                version,
                source: source
                    .map(|text| serde_json::from_str(&text))
                    .transpose()
                    .context("invalid recorded source descriptor")?,
                archive_blake3,
                catalog: catalog
                    .map(|text| serde_json::from_str(&text))
                    .transpose()
                    .context("invalid recorded catalog reference")?,
                declarations: declarations
                    .map(|text| serde_json::from_str(&text))
                    .transpose()
                    .context("invalid recorded release declarations")?,
            })
        },
    )
    .transpose()
}

/// Add the nullable catalog and declaration columns to older manager state.
pub(crate) fn ensure_schema(connection: &Connection) -> Result<()> {
    for name in ["catalog", "declarations"] {
        if !has_column(connection, "managed_package_releases", name)? {
            connection.execute_batch(&format!(
                "ALTER TABLE managed_package_releases ADD COLUMN {name} TEXT;"
            ))?;
        }
    }
    Ok(())
}

fn has_table(connection: &Connection, name: &str) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [name],
        |row| row.get(0),
    )?)
}

fn has_column(connection: &Connection, table: &str, name: &str) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name=?2)",
        params![table, name],
        |row| row.get(0),
    )?)
}
