use super::{
    FileSpec, HistoryContext, HistoryFile, HistoryKind, HistoryOperation, HistoryPackage,
    HistoryRelease, ReleaseMetadata,
};
use crate::transactions::CatalogReference;
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::collections::BTreeMap;

type PackageReleaseRow = (String, Option<String>, Option<String>, Option<String>);

/// Release metadata for one side of an operation.
pub(crate) fn metadata_for(
    context: &HistoryContext,
    package_id: &str,
    reference: Option<&CatalogReference>,
    version: Option<&str>,
    archive_blake3: Option<&str>,
) -> ReleaseMetadata {
    let standalone = if reference.is_none() {
        context.standalone.as_ref()
    } else {
        None
    };
    ReleaseMetadata {
        mod_id: reference
            .map(|reference| reference.mod_id.clone())
            .or_else(|| standalone.map(|package| package.mod_id.clone()))
            .unwrap_or_else(|| package_id.to_owned()),
        version: version
            .map(str::to_owned)
            .or_else(|| standalone.and_then(|package| package.version.clone())),
        source: reference
            .and_then(|reference| context.sources.get(&reference.to_string()).cloned())
            .or_else(|| standalone.map(|package| package.source.clone())),
        archive_blake3: archive_blake3.map(str::to_owned),
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
    tx.execute(
        "INSERT OR REPLACE INTO managed_package_releases
             (package_id, mod_id, version, source, archive_blake3)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            package_id,
            metadata.mod_id,
            metadata.version,
            source,
            metadata.archive_blake3
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
    let row: Option<PackageReleaseRow> = connection
        .query_row(
            "SELECT mod_id, version, source, archive_blake3
             FROM managed_package_releases WHERE package_id=?1",
            [package_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    row.map(
        |(mod_id, version, source, archive_blake3)| -> Result<ReleaseMetadata> {
            Ok(ReleaseMetadata {
                mod_id,
                version,
                source: source
                    .map(|text| serde_json::from_str(&text))
                    .transpose()
                    .context("invalid recorded source descriptor")?,
                archive_blake3,
            })
        },
    )
    .transpose()
}

/// Snapshot an installed package before its ownership rows are removed.
pub(crate) fn package_snapshot(
    connection: &Connection,
    package_id: &str,
) -> Result<HistoryPackage> {
    let archive_sha256: String = connection.query_row(
        "SELECT archive_sha256 FROM managed_packages WHERE id=?1",
        [package_id],
        |row| row.get(0),
    )?;
    let metadata = package_metadata(connection, package_id)?;
    let mut statement = connection.prepare(
        "SELECT root, relative_path, sha256 FROM managed_files
         WHERE package_id=?1 ORDER BY root, path_key",
    )?;
    let files = statement
        .query_map([package_id], |row| {
            Ok(HistoryFile {
                root: row.get(0)?,
                destination: row.get(1)?,
                digest_before: Some(row.get(2)?),
                digest_after: None,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let (mod_id, version, source, archive_blake3) = metadata.map_or_else(
        || (package_id.to_owned(), None, None, None),
        |metadata| {
            (
                metadata.mod_id,
                metadata.version,
                metadata.source,
                metadata.archive_blake3,
            )
        },
    );
    Ok(HistoryPackage {
        mod_id,
        package_id_before: Some(package_id.to_owned()),
        package_id_after: None,
        release_before: Some(HistoryRelease {
            version,
            source,
            archive_sha256: Some(archive_sha256),
            archive_blake3,
        }),
        release_after: None,
        files,
    })
}

/// Build the record of a committed install or update.
pub(crate) fn install_operation(
    context: &HistoryContext,
    operation_id: &str,
    package_id: &str,
    metadata: ReleaseMetadata,
    archive_sha256: &str,
    files: &[FileSpec],
    previous: Option<HistoryPackage>,
) -> HistoryOperation {
    let mut merged: BTreeMap<(String, String), HistoryFile> = BTreeMap::new();
    if let Some(previous) = &previous {
        for file in &previous.files {
            merged.insert((file.root.clone(), file.destination.clone()), file.clone());
        }
    }
    for file in files {
        let entry = merged
            .entry((file.root.clone(), file.destination.clone()))
            .or_insert_with(|| HistoryFile {
                root: file.root.clone(),
                destination: file.destination.clone(),
                digest_before: None,
                digest_after: None,
            });
        entry.digest_after = Some(file.digest.clone());
    }
    let kind = if previous.is_some() {
        HistoryKind::Update
    } else {
        HistoryKind::Install
    };
    HistoryOperation {
        id: operation_id.to_owned(),
        kind,
        game_id: context.game_id.clone(),
        installation: context.installation.clone(),
        recorded_at: String::new(),
        packages: vec![HistoryPackage {
            mod_id: metadata.mod_id,
            package_id_before: previous
                .as_ref()
                .and_then(|package| package.package_id_before.clone()),
            package_id_after: Some(package_id.to_owned()),
            release_before: previous.and_then(|package| package.release_before),
            release_after: Some(HistoryRelease {
                version: metadata.version,
                source: metadata.source,
                archive_sha256: Some(archive_sha256.to_owned()),
                archive_blake3: metadata.archive_blake3,
            }),
            files: merged.into_values().collect(),
        }],
    }
}

/// Build the record of a committed uninstall.
pub(crate) fn uninstall_operation(
    context: &HistoryContext,
    operation_id: &str,
    package: HistoryPackage,
) -> HistoryOperation {
    HistoryOperation {
        id: operation_id.to_owned(),
        kind: HistoryKind::Uninstall,
        game_id: context.game_id.clone(),
        installation: context.installation.clone(),
        recorded_at: String::new(),
        packages: vec![package],
    }
}

/// Build the record of a committed installation stack.
pub(crate) fn stack_operation(
    context: &HistoryContext,
    operation_id: &str,
    packages: &[crate::transactions::ManagedPackage],
) -> HistoryOperation {
    let packages = packages
        .iter()
        .map(|package| {
            let mut files: Vec<HistoryFile> = package
                .files
                .iter()
                .map(|file| HistoryFile {
                    root: file.root.clone(),
                    destination: file.destination.clone(),
                    digest_before: None,
                    digest_after: Some(file.sha256.clone()),
                })
                .collect();
            files.sort_by(|a, b| (&a.root, &a.destination).cmp(&(&b.root, &b.destination)));
            HistoryPackage {
                mod_id: package.mod_id.clone().unwrap_or_else(|| package.id.clone()),
                package_id_before: None,
                package_id_after: Some(package.id.clone()),
                release_before: None,
                release_after: Some(HistoryRelease {
                    version: package.version.clone(),
                    source: package.source.clone(),
                    archive_sha256: Some(package.archive_sha256.clone()),
                    archive_blake3: package.archive_blake3.clone(),
                }),
                files,
            }
        })
        .collect();
    HistoryOperation {
        id: operation_id.to_owned(),
        kind: HistoryKind::Install,
        game_id: context.game_id.clone(),
        installation: context.installation.clone(),
        recorded_at: String::new(),
        packages,
    }
}

/// Write `operation` inside the caller's ownership transaction.
pub(crate) fn insert(tx: &Transaction<'_>, operation: &HistoryOperation) -> Result<()> {
    let recorded_at = (!operation.recorded_at.is_empty()).then_some(operation.recorded_at.as_str());
    tx.execute(
        "INSERT INTO history_operations(id, kind, game_id, installation, recorded_at)
         VALUES (?1, ?2, ?3, ?4, COALESCE(?5, strftime('%Y-%m-%dT%H:%M:%fZ','now')))",
        params![
            operation.id,
            operation.kind.as_str(),
            operation.game_id,
            operation.installation.to_string_lossy(),
            recorded_at
        ],
    )?;
    for (ordinal, package) in operation.packages.iter().enumerate() {
        let before = package
            .release_before
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let after = package
            .release_after
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        tx.execute(
            "INSERT INTO history_packages
                 (operation_id, ord, mod_id, package_id_before, package_id_after,
                  release_before, release_after)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                operation.id,
                ordinal as i64,
                package.mod_id,
                package.package_id_before,
                package.package_id_after,
                before,
                after
            ],
        )?;
        for (index, file) in package.files.iter().enumerate() {
            tx.execute(
                "INSERT INTO history_files
                     (operation_id, package_ord, ord, root, relative_path,
                      digest_before, digest_after)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    operation.id,
                    ordinal as i64,
                    index as i64,
                    file.root,
                    file.destination,
                    file.digest_before,
                    file.digest_after
                ],
            )?;
        }
    }
    Ok(())
}

/// Read committed operations, newest first, optionally for one game ID.
pub(crate) fn read(
    connection: &Connection,
    game_id: Option<&str>,
) -> Result<Vec<HistoryOperation>> {
    if !has_table(connection, "history_operations")? {
        return Ok(Vec::new());
    }
    let mut statement = connection.prepare(
        "SELECT id, kind, game_id, installation, recorded_at
         FROM history_operations
         WHERE ?1 IS NULL OR game_id=?1
         ORDER BY seq DESC",
    )?;
    let rows = statement
        .query_map([game_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(id, kind, game_id, installation, recorded_at)| {
            let kind = HistoryKind::from_label(&kind)
                .with_context(|| format!("unknown history kind {kind}"))?;
            let mut package_statement = connection.prepare(
                "SELECT ord, mod_id, package_id_before, package_id_after,
                        release_before, release_after
                 FROM history_packages WHERE operation_id=?1 ORDER BY ord",
            )?;
            let packages = package_statement
                .query_map([&id], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut package_rows = Vec::new();
            for (ordinal, mod_id, before_id, after_id, before, after) in packages {
                let before = before
                    .map(|text| serde_json::from_str(&text))
                    .transpose()
                    .context("invalid history release")?;
                let after = after
                    .map(|text| serde_json::from_str(&text))
                    .transpose()
                    .context("invalid history release")?;
                let mut file_statement = connection.prepare(
                    "SELECT root, relative_path, digest_before, digest_after
                     FROM history_files
                     WHERE operation_id=?1 AND package_ord=?2 ORDER BY ord",
                )?;
                let files = file_statement
                    .query_map(params![id, ordinal], |row| {
                        Ok(HistoryFile {
                            root: row.get(0)?,
                            destination: row.get(1)?,
                            digest_before: row.get(2)?,
                            digest_after: row.get(3)?,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                package_rows.push(HistoryPackage {
                    mod_id,
                    package_id_before: before_id,
                    package_id_after: after_id,
                    release_before: before,
                    release_after: after,
                    files,
                });
            }
            Ok(HistoryOperation {
                id,
                kind,
                game_id,
                installation: std::path::PathBuf::from(installation),
                recorded_at,
                packages: package_rows,
            })
        })
        .collect()
}

fn has_table(connection: &Connection, name: &str) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [name],
        |row| row.get(0),
    )?)
}
