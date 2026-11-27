//! SQLite commit phase for install and uninstall transactions.
use super::journal::{JournalDirectory, JournalFile};
use super::profile_switch::{PreparedSwitch, Remove, Write};
use crate::state::{record_root, RootKind, Store};
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection};
use std::collections::BTreeMap;
use std::path::Path;

impl Store {
    #[expect(
        clippy::too_many_arguments,
        reason = "the commit names the root, catalog and stack context separately"
    )]
    pub(super) fn commit_install(
        &mut self,
        id: &str,
        game: &Path,
        hash: &str,
        files: &[JournalFile],
        directories: &[JournalDirectory],
        content: Option<&Path>,
        catalog: Option<&super::catalog::CatalogRecord>,
        archive_blake3: Option<&str>,
        stack_id: Option<&str>,
    ) -> Result<()> {
        let context = self.history.clone();
        self.committed_operation = None;
        let tx = self.connection.transaction()?;
        let replaced = catalog.and_then(|catalog| catalog.replacing);
        let metadata = context.as_ref().map(|context| {
            crate::history::metadata_for(
                context,
                id,
                catalog.map(|catalog| catalog.release),
                archive_blake3,
            )
        });
        let record = match (context.as_ref(), metadata.as_ref()) {
            (Some(context), Some(metadata)) if stack_id.is_none() => {
                let previous = match replaced {
                    Some(replaced) => {
                        Some(crate::history::package_snapshot(&tx, replaced.package_id)?)
                    }
                    None => None,
                };
                let specs: Vec<crate::history::FileSpec> = files
                    .iter()
                    .map(|file| crate::history::FileSpec {
                        root: file.root.label().to_owned(),
                        destination: file.destination.clone(),
                        digest: file.blake3.clone(),
                    })
                    .collect();
                Some(crate::history::install_operation(
                    context,
                    id,
                    id,
                    metadata.clone(),
                    hash,
                    &specs,
                    previous,
                ))
            }
            _ => None,
        };
        if let Some(replaced) = replaced {
            super::update::vacate(&tx, replaced)?;
        }
        record_root(&tx, game, RootKind::Game, game)?;
        if let Some(content) = content {
            record_root(&tx, game, RootKind::Content, content)?;
        }
        tx.execute(
            "INSERT INTO managed_packages(id,game_path,archive_blake3) VALUES (?1,?2,?3)",
            params![id, game.to_string_lossy().as_ref(), hash],
        )?;
        let profile = crate::state::profiles::ensure_profile(&tx, game)?;
        let moved = match replaced {
            Some(replaced) => {
                crate::state::profiles::transfer_memberships(&tx, replaced.package_id, id)?
            }
            None => 0,
        };
        if moved == 0 {
            crate::state::profiles::append_package(&tx, &profile.id, id)?;
        }
        if let Some(metadata) = &metadata {
            crate::history::record_metadata(&tx, id, metadata)?;
        }
        for f in files {
            let path_key = super::ownership_key(&f.destination);
            tx.execute(
                "DELETE FROM managed_files
                 WHERE game_path=?1 AND root=?2 AND path_key=?3 AND package_id<>?4",
                params![
                    game.to_string_lossy().as_ref(),
                    f.root.label(),
                    path_key,
                    id
                ],
            )?;
            tx.execute("INSERT INTO managed_files(game_path,root,path_key,relative_path,package_id,blake3,bytes,profile_id,archive_entry)
                VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![game.to_string_lossy().as_ref(), f.root.label(),
                    path_key,
                    f.destination, id, f.blake3, f.bytes as i64, profile.id, f.archive_entry])?;
            tx.execute(
                "INSERT OR REPLACE INTO file_claims
                 (package_id,game_path,root,path_key,relative_path,archive_entry,blake3,bytes)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    id,
                    game.to_string_lossy().as_ref(),
                    f.root.label(),
                    path_key,
                    f.destination,
                    f.archive_entry,
                    f.blake3,
                    f.bytes as i64
                ],
            )?;
        }
        for directory in directories {
            let identity: String = tx.query_row(
                "SELECT identity FROM pending_directories WHERE transaction_id=?1 AND root=?2 AND relative_path=?3",
                params![id, directory.root.label(), directory.path],
                |row| row.get::<_, Option<String>>(0),
            )?.context("exclusive directory identity must be persisted before commit")?;
            tx.execute(
                "INSERT INTO managed_directories(game_path,root,path_key,relative_path,package_id,identity)
                 VALUES (?1,?2,?3,?4,?5,?6)",
                params![game.to_string_lossy().as_ref(), directory.root.label(),
                    super::ownership_key(&directory.path), directory.path, id, identity],
            )?;
        }
        if let Some(catalog) = catalog {
            super::catalog::record(&tx, id, catalog)?;
        }
        if let Some(stack_id) = stack_id {
            tx.execute(
                "INSERT INTO pending_stack_packages(stack_id,ord,package_id)
                 SELECT ?1, COUNT(*), ?2 FROM pending_stack_packages WHERE stack_id=?1",
                params![stack_id, id],
            )?;
        }
        if let Some(replaced) = replaced {
            super::update::retire(&tx, replaced, id)?;
        }
        super::created_dirs::commit(&tx, id, game, content)?;
        if let Some(record) = &record {
            crate::history::insert(&tx, record)?;
        }
        tx.execute(
            "UPDATE pending_transactions SET status='committed' WHERE id=?1",
            [id],
        )?;
        tx.commit()?;
        self.committed_operation = record.map(|record| record.id);
        Ok(())
    }

    pub(super) fn commit_uninstall(&mut self, journal_id: &str, package_id: &str) -> Result<()> {
        self.commit_uninstall_many(&[(journal_id.to_owned(), package_id.to_owned())])
    }

    pub(crate) fn commit_uninstall_many(&mut self, removals: &[(String, String)]) -> Result<()> {
        let context = self.history.clone();
        self.committed_operation = None;
        let tx = self.connection.transaction()?;
        let record = match (context.as_ref(), removals) {
            (Some(context), [(journal_id, package_id)]) => {
                let package = crate::history::package_snapshot(&tx, package_id)?;
                Some(crate::history::uninstall_operation(
                    context, journal_id, package,
                ))
            }
            (Some(context), _) => {
                let mut packages = Vec::new();
                for (_, package_id) in removals {
                    packages.push(crate::history::package_snapshot(&tx, package_id)?);
                }
                Some(crate::history::uninstall_operation_many(
                    context,
                    &removals[0].0,
                    packages,
                ))
            }
            (None, _) => None,
        };
        for (journal_id, package_id) in removals {
            super::restore_recovery::move_winners(&tx, journal_id, package_id)?;
            crate::state::profiles::delete_package_records(&tx, package_id)?;
            tx.execute("DELETE FROM managed_packages WHERE id=?1", [package_id])?;
            let committed = tx.execute(
                "UPDATE pending_transactions SET status='committed' WHERE id=?1 AND status='pending'",
                [journal_id],
            )?;
            if committed != 1 {
                bail!("removal journal {journal_id} is no longer pending; run 'modn recover'");
            }
        }
        if let Some(record) = &record {
            crate::history::insert(&tx, record)?;
        }
        tx.commit()?;
        self.committed_operation = record.map(|record| record.id);
        Ok(())
    }

    /// Commit one profile switch: the active profile, the winner rows, the
    /// history record and the journal status in one SQLite transaction.
    pub(super) fn commit_profile_switch(
        &mut self,
        id: &str,
        prepared: &PreparedSwitch,
    ) -> Result<String> {
        let record = switch_operation(&self.connection, id, prepared)?;
        let game = prepared.game.to_string_lossy();
        let tx = self.connection.transaction()?;
        tx.execute(
            "UPDATE profiles SET active=0 WHERE game_path=?1 AND active=1",
            [game.as_ref()],
        )?;
        let activated = tx.execute(
            "UPDATE profiles SET active=1 WHERE id=?1",
            [&prepared.to.id],
        )?;
        if activated != 1 {
            bail!(
                "profile {} disappeared before the switch committed; run 'modn doctor'",
                prepared.to.name
            );
        }
        tx.execute(
            "DELETE FROM managed_files WHERE game_path=?1 AND profile_id=?2",
            params![game.as_ref(), &prepared.from.id],
        )?;
        for winner in &prepared.winners {
            tx.execute(
                "INSERT INTO managed_files
                 (game_path,root,path_key,relative_path,package_id,blake3,bytes,profile_id,archive_entry)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    game.as_ref(),
                    winner.root.label(),
                    winner.path_key,
                    winner.relative_path,
                    winner.package_id,
                    winner.blake3,
                    winner.bytes as i64,
                    prepared.to.id,
                    winner.archive_entry
                ],
            )?;
        }
        crate::history::insert(&tx, &record)?;
        tx.execute(
            "INSERT INTO history_profile_switches(operation_id,game_path,from_profile,to_profile)
             VALUES (?1,?2,?3,?4)",
            params![id, game.as_ref(), prepared.from.name, prepared.to.name],
        )?;
        let committed = tx.execute(
            "UPDATE pending_transactions SET status='committed' WHERE id=?1 AND status='pending'",
            [id],
        )?;
        if committed != 1 {
            bail!("profile switch journal {id} is no longer pending; run 'modn recover'");
        }
        tx.commit()?;
        self.committed_operation = Some(record.id.clone());
        Ok(record.id)
    }
}

/// Build the history record of one profile switch.
fn switch_operation(
    connection: &Connection,
    id: &str,
    prepared: &PreparedSwitch,
) -> Result<crate::history::HistoryOperation> {
    let mut packages: BTreeMap<String, Changed> = BTreeMap::new();
    for remove in &prepared.removes {
        record_file(&mut packages, remove);
    }
    for write in &prepared.writes {
        record_write(&mut packages, write);
    }
    let mut rows = Vec::new();
    for (package_id, changed) in packages {
        let metadata = crate::history::package_metadata(connection, &package_id)?;
        let mod_id = metadata.map_or_else(|| package_id.clone(), |metadata| metadata.mod_id);
        rows.push(crate::history::HistoryPackage {
            mod_id,
            package_id_before: changed.before.then(|| package_id.clone()),
            package_id_after: changed.after.then(|| package_id.clone()),
            release_before: None,
            release_after: None,
            files: changed.files.into_values().collect(),
        });
    }
    Ok(crate::history::HistoryOperation {
        id: id.to_owned(),
        kind: crate::history::HistoryKind::ProfileSwitch,
        game_id: prepared.plan.game.clone(),
        installation: prepared.game.clone(),
        recorded_at: String::new(),
        packages: rows,
        undoes: None,
    })
}

/// One package whose files a switch removes, writes or both.
#[derive(Default)]
struct Changed {
    before: bool,
    after: bool,
    files: BTreeMap<(String, String), crate::history::HistoryFile>,
}

fn record_file(packages: &mut BTreeMap<String, Changed>, remove: &Remove) {
    let changed = packages.entry(remove.package_id.clone()).or_default();
    changed.before = true;
    let file = changed
        .files
        .entry((remove.root.label().to_owned(), remove.relative_path.clone()))
        .or_insert_with(|| history_file(remove.root.label(), &remove.relative_path));
    file.digest_before = Some(remove.blake3.clone());
}

fn record_write(packages: &mut BTreeMap<String, Changed>, write: &Write) {
    let changed = packages.entry(write.package_id.clone()).or_default();
    changed.after = true;
    let file = changed
        .files
        .entry((write.root.label().to_owned(), write.relative_path.clone()))
        .or_insert_with(|| history_file(write.root.label(), &write.relative_path));
    if let Some(old) = &write.replaces_blake3 {
        file.digest_before = Some(old.clone());
    }
    file.digest_after = Some(write.blake3.clone());
    let (Some(owner), Some(old)) = (&write.replaces, &write.replaces_blake3) else {
        return;
    };
    let replaced = packages.entry(owner.clone()).or_default();
    replaced.before = true;
    let file = replaced
        .files
        .entry((write.root.label().to_owned(), write.relative_path.clone()))
        .or_insert_with(|| history_file(write.root.label(), &write.relative_path));
    file.digest_before = Some(old.clone());
}

fn history_file(root: &str, destination: &str) -> crate::history::HistoryFile {
    crate::history::HistoryFile {
        root: root.to_owned(),
        destination: destination.to_owned(),
        digest_before: None,
        digest_after: None,
    }
}
