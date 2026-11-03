use super::directory::case_key;
use super::journal::target_path;
use super::validation::validate_transaction_root;
use crate::state::Store;
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
    pub(super) fn create_parents(
        &mut self,
        transaction_id: &str,
        game: &Path,
        relative: &str,
    ) -> Result<()> {
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
                "INSERT INTO pending_created_directories(transaction_id,ord,relative_path)
                 SELECT ?1, COALESCE(MAX(ord) + 1, 0), ?2
                 FROM pending_created_directories WHERE transaction_id=?1",
                params![transaction_id, directory],
            )?;
            match fs::create_dir(&current) {
                Ok(()) => match super::directory::identity(&current) {
                    Ok(identity) => {
                        self.connection.execute(
                            "UPDATE pending_created_directories SET identity=?3
                             WHERE transaction_id=?1 AND relative_path=?2",
                            params![transaction_id, directory, identity],
                        )?;
                    }
                    Err(_) => {
                        self.connection.execute(
                            "DELETE FROM pending_created_directories
                             WHERE transaction_id=?1 AND relative_path=?2",
                            params![transaction_id, directory],
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
                bail!("destination escapes game root");
            }
        }
        Ok(())
    }

    pub(super) fn remove_journal_directories(
        &self,
        game: &Path,
        transaction_id: &str,
    ) -> Result<()> {
        let mut statement = self.connection.prepare(
            "SELECT relative_path, identity FROM pending_created_directories
             WHERE transaction_id=?1 ORDER BY ord DESC",
        )?;
        let created = statement
            .query_map([transaction_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get(1)?))
            })?
            .collect::<rusqlite::Result<Vec<(String, Option<String>)>>>()?;
        drop(statement);
        let game_path = game.to_string_lossy().into_owned();
        for (relative, identity) in &created {
            if prune_one(game, relative, identity.as_deref())? == Prune::Occupied {
                self.connection.execute(
                    "INSERT OR REPLACE INTO created_directories
                         (game_path,path_key,relative_path,identity) VALUES (?1,?2,?3,?4)",
                    params![game_path, case_key(relative), relative, identity],
                )?;
            }
        }
        Ok(())
    }

    pub(super) fn prune_created(&mut self, game: &Path) -> Result<usize> {
        let root = fs::symlink_metadata(game)?;
        if !root.is_dir() || root.file_type().is_symlink() {
            bail!("game root is not a real directory: {}", game.display());
        }
        let game_path = game.to_string_lossy().into_owned();
        let mut statement = self.connection.prepare(
            "SELECT path_key, relative_path, identity FROM created_directories
             WHERE game_path=?1",
        )?;
        let mut recorded = statement
            .query_map([&game_path], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<(String, String, Option<String>)>>>()?;
        drop(statement);
        recorded
            .sort_by_key(|(key, ..)| std::cmp::Reverse((key.matches('/').count(), key.clone())));
        let mut removed = 0;
        for (key, relative, identity) in &recorded {
            let outcome = prune_one(game, relative, identity.as_deref())?;
            if outcome == Prune::Removed {
                removed += 1;
            }
            if outcome != Prune::Occupied {
                self.connection.execute(
                    "DELETE FROM created_directories WHERE game_path=?1 AND path_key=?2",
                    params![game_path, key],
                )?;
            }
        }
        Ok(removed)
    }

    pub(super) fn prune_all_created(&mut self) -> Result<usize> {
        let mut statement = self
            .connection
            .prepare("SELECT DISTINCT game_path FROM created_directories ORDER BY game_path")?;
        let games = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        let mut removed = 0;
        for game in games {
            let stored = PathBuf::from(game);
            match validate_transaction_root(&stored) {
                Ok(root) if root == stored => removed += self.prune_created(&root)?,
                _ => {}
            }
        }
        Ok(removed)
    }
}

pub(super) fn commit(tx: &Transaction<'_>, transaction_id: &str, game: &Path) -> Result<()> {
    let mut statement = tx.prepare(
        "SELECT relative_path, identity FROM pending_created_directories
         WHERE transaction_id=?1 ORDER BY ord",
    )?;
    let created = statement
        .query_map([transaction_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get(1)?))
        })?
        .collect::<rusqlite::Result<Vec<(String, Option<String>)>>>()?;
    let game_path = game.to_string_lossy();
    for (relative, identity) in &created {
        let Some(identity) = identity else {
            continue;
        };
        tx.execute(
            "INSERT OR REPLACE INTO created_directories(game_path,path_key,relative_path,identity)
             VALUES (?1,?2,?3,?4)",
            params![game_path.as_ref(), case_key(relative), relative, identity],
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
