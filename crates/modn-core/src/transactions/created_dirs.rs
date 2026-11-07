use super::directory::case_key;
use super::journal::{target_path, Roots};
use crate::state::{resolve_root, RootKind, Store};
use anyhow::{bail, Context, Result};
use rusqlite::{params, Transaction};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
enum Prune {
    Removed,
    Missing,
    Replaced,
    Occupied,
    Unproven,
}

impl Store {
    /// Create a missing content root below `base`, recording every directory
    /// this transaction creates so rollback removes only Modn's own folders.
    pub(super) fn ensure_content_root(
        &mut self,
        transaction_id: &str,
        roots: &Roots,
        base: Option<&Path>,
    ) -> Result<()> {
        let Some(content) = roots.content.as_deref() else {
            return Ok(());
        };
        if fs::symlink_metadata(content).is_ok() {
            return Ok(());
        }
        let base = base.context("content root has no recorded base")?;
        let relative = content
            .strip_prefix(base)
            .context("content root is not below its base")?;
        let mut current = base.to_path_buf();
        let mut recorded = String::new();
        for component in relative.components() {
            let std::path::Component::Normal(part) = component else {
                bail!("invalid content root component");
            };
            current.push(part);
            if !recorded.is_empty() {
                recorded.push('/');
            }
            recorded.push_str(&part.to_string_lossy());
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => continue,
                Ok(_) => bail!("unsafe content root component: {}", current.display()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            fs::create_dir(&current)?;
            let identity = super::directory::identity(&current).ok();
            self.connection.execute(
                "INSERT INTO pending_created_directories
                     (transaction_id,ord,root,relative_path,identity,base_path)
                 SELECT ?1, COALESCE(MAX(ord) + 1, 0), 'appdata', ?2, ?3, ?4
                 FROM pending_created_directories WHERE transaction_id=?1",
                params![
                    transaction_id,
                    recorded,
                    identity,
                    base.to_string_lossy().as_ref()
                ],
            )?;
        }
        Ok(())
    }

    pub(super) fn create_parents(
        &mut self,
        transaction_id: &str,
        roots: &Roots,
        kind: RootKind,
        relative: &str,
    ) -> Result<()> {
        let game = roots.path(kind)?;
        target_path(game, relative)?;
        let canonical = game.canonicalize()?;
        let parts: Vec<&str> = relative.split('/').collect();
        let mut current = canonical.clone();
        let mut directory = String::new();
        for part in &parts[..parts.len() - 1] {
            current.push(part);
            if !directory.is_empty() {
                directory.push('/');
            }
            directory.push_str(part);
            match fs::symlink_metadata(&current) {
                Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => continue,
                Ok(_) => bail!("linked or non-directory ancestor: {}", current.display()),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            self.connection.execute(
                "INSERT INTO pending_created_directories(transaction_id,ord,root,relative_path)
                 SELECT ?1, COALESCE(MAX(ord) + 1, 0), ?2, ?3
                 FROM pending_created_directories WHERE transaction_id=?1",
                params![transaction_id, kind.label(), directory],
            )?;
            match fs::create_dir(&current) {
                Ok(()) => match super::directory::identity(&current) {
                    Ok(identity) => {
                        self.connection.execute(
                            "UPDATE pending_created_directories SET identity=?4
                             WHERE transaction_id=?1 AND root=?2 AND relative_path=?3",
                            params![transaction_id, kind.label(), directory, identity],
                        )?;
                    }
                    Err(_) => {
                        self.connection.execute(
                            "DELETE FROM pending_created_directories
                             WHERE transaction_id=?1 AND root=?2 AND relative_path=?3",
                            params![transaction_id, kind.label(), directory],
                        )?;
                    }
                },
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    self.connection.execute(
                        "DELETE FROM pending_created_directories
                         WHERE transaction_id=?1 AND relative_path=?2",
                        params![transaction_id, directory],
                    )?;
                    let meta = fs::symlink_metadata(&current)?;
                    if !meta.is_dir() || meta.file_type().is_symlink() {
                        bail!("linked or non-directory ancestor: {}", current.display());
                    }
                }
                Err(e) => {
                    return Err(e).with_context(|| format!("cannot create {}", current.display()))
                }
            }
            if !current.canonicalize()?.starts_with(&canonical) {
                bail!("destination escapes its installation root");
            }
        }
        Ok(())
    }

    pub(super) fn remove_journal_directories(
        &self,
        roots: &Roots,
        transaction_id: &str,
    ) -> Result<()> {
        let mut statement = self.connection.prepare(
            "SELECT root, relative_path, identity, base_path FROM pending_created_directories
             WHERE transaction_id=?1 ORDER BY ord DESC",
        )?;
        let created: Vec<(String, String, Option<String>, Option<String>)> = statement
            .query_map([transaction_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        for (root, relative, identity, base_path) in &created {
            let (recorded_path, base) = match base_path {
                Some(base) => (base.clone(), PathBuf::from(base)),
                None => {
                    let kind = RootKind::from_label(root).context("unknown created root")?;
                    let path = roots.path(kind)?.to_string_lossy().into_owned();
                    (path.clone(), PathBuf::from(path))
                }
            };
            if prune_one(&base, relative, identity.as_deref())? == Prune::Occupied {
                self.connection.execute(
                    "INSERT OR REPLACE INTO created_directories
                         (game_path,root,path_key,relative_path,identity)
                         VALUES (?1,?2,?3,?4,?5)",
                    params![recorded_path, root, case_key(relative), relative, identity],
                )?;
            }
        }
        Ok(())
    }

    pub(super) fn prune_created(&mut self, roots: &Roots) -> Result<usize> {
        let mut removed = 0;
        for kind in [RootKind::Game, RootKind::Content] {
            let Ok(root) = roots.path(kind) else {
                continue;
            };
            let metadata = fs::symlink_metadata(root)?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                bail!(
                    "installation root is not a real directory: {}",
                    root.display()
                );
            }
            let game_path = roots.game.to_string_lossy().into_owned();
            let mut statement = self.connection.prepare(
                "SELECT path_key, relative_path, identity FROM created_directories
                 WHERE game_path=?1 AND root=?2",
            )?;
            let mut recorded = statement
                .query_map(params![game_path, kind.label()], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<(String, String, Option<String>)>>>()?;
            drop(statement);
            recorded.sort_by_key(|(key, ..)| {
                std::cmp::Reverse((key.matches('/').count(), key.clone()))
            });
            for (key, relative, identity) in &recorded {
                let outcome = prune_one(root, relative, identity.as_deref())?;
                if outcome == Prune::Removed {
                    removed += 1;
                }
                if outcome != Prune::Occupied {
                    self.connection.execute(
                        "DELETE FROM created_directories
                         WHERE game_path=?1 AND root=?2 AND path_key=?3",
                        params![game_path, kind.label(), key],
                    )?;
                }
            }
        }
        Ok(removed)
    }

    pub(super) fn prune_all_created(&mut self) -> Result<usize> {
        let mut statement = self.connection.prepare(
            "SELECT DISTINCT game_path, root FROM created_directories ORDER BY game_path, root",
        )?;
        let roots = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        let mut removed = 0;
        for (stored, label) in roots {
            let base = PathBuf::from(&stored);
            if label == "appdata" {
                removed += self.prune_created_labeled(
                    &Roots {
                        game: base.clone(),
                        content: None,
                    },
                    &label,
                    &base,
                )?;
            } else if let Some(kind) = RootKind::from_label(&label) {
                if let Ok(physical) = resolve_root(&self.connection, &base, kind) {
                    removed += self.prune_created_labeled(
                        &Roots {
                            game: base,
                            content: None,
                        },
                        &label,
                        &physical,
                    )?;
                }
            }
        }
        Ok(removed)
    }

    fn prune_created_labeled(
        &mut self,
        roots: &Roots,
        label: &str,
        physical: &Path,
    ) -> Result<usize> {
        let mut statement = self.connection.prepare(
            "SELECT path_key, relative_path, identity FROM created_directories
             WHERE game_path=?1 AND root=?2",
        )?;
        let mut recorded = statement
            .query_map(params![roots.game.to_string_lossy(), label], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<(String, String, Option<String>)>>>()?;
        drop(statement);
        recorded
            .sort_by_key(|(key, ..)| std::cmp::Reverse((key.matches('/').count(), key.clone())));
        let mut removed = 0;
        for (key, relative, identity) in &recorded {
            let outcome = prune_one(physical, relative, identity.as_deref())?;
            if outcome == Prune::Removed {
                removed += 1;
            }
            if outcome != Prune::Occupied {
                self.connection.execute(
                    "DELETE FROM created_directories
                     WHERE game_path=?1 AND root=?2 AND path_key=?3",
                    params![roots.game.to_string_lossy(), label, key],
                )?;
            }
        }
        Ok(removed)
    }
}

pub(super) fn commit(
    tx: &Transaction<'_>,
    transaction_id: &str,
    game: &Path,
    _content: Option<&Path>,
) -> Result<()> {
    let mut statement = tx.prepare(
        "SELECT root, relative_path, identity, base_path FROM pending_created_directories
         WHERE transaction_id=?1 ORDER BY ord",
    )?;
    let created: Vec<(String, String, Option<String>, Option<String>)> = statement
        .query_map([transaction_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let game_path = game.to_string_lossy();
    for (root, relative, identity, base_path) in &created {
        let Some(identity) = identity else {
            continue;
        };
        let owner = base_path.as_deref().unwrap_or(game_path.as_ref());
        tx.execute(
            "INSERT OR REPLACE INTO created_directories
                 (game_path,root,path_key,relative_path,identity) VALUES (?1,?2,?3,?4,?5)",
            params![owner, root, case_key(relative), relative, identity],
        )?;
    }
    Ok(())
}

fn prune_one(game: &Path, relative: &str, identity: Option<&str>) -> Result<Prune> {
    let mut path = game.to_path_buf();
    for part in relative.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains(['\\', ':']) {
            bail!("invalid recorded directory: {relative}");
        }
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) => return Ok(Prune::Replaced),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Prune::Missing),
            Err(e) => return Err(e.into()),
        }
    }
    let Some(expected) = identity else {
        return Ok(Prune::Unproven);
    };
    if super::directory::identity(&path).ok().as_deref() != Some(expected) {
        return Ok(Prune::Replaced);
    }
    if fs::read_dir(&path)?.next().is_some() {
        return Ok(Prune::Occupied);
    }
    match fs::remove_dir(&path) {
        Ok(()) => Ok(Prune::Removed),
        Err(e) if e.kind() == io::ErrorKind::DirectoryNotEmpty => Ok(Prune::Occupied),
        Err(e) => Err(e).with_context(|| format!("cannot remove {}", path.display())),
    }
}
