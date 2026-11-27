//! Named per-game profiles and their package priority order.
//!
//! Every game installation has at least one profile, and exactly one profile is
//! active. A package belongs to a profile through a membership that records its
//! priority and enabled flag. A higher priority wins a file conflict, so the
//! package at rank 1 is the winner. New installs append at the next priority.

use crate::error::{coded, ErrorCode};
use crate::state::Store;
use crate::transactions::lock_store;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::path::Path;

/// One named profile of a game installation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Profile {
    /// Stable profile identifier.
    pub id: String,
    /// User-facing name, unique within the installation.
    pub name: String,
    /// Stored creation time, exactly as SQLite wrote it.
    pub created_at: String,
    /// Whether this profile decides the files on disk.
    pub active: bool,
    /// Memberships ordered from the winner to the lowest priority.
    pub packages: Vec<ProfilePackage>,
}

/// One package's membership in a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProfilePackage {
    /// Managed package identifier.
    pub package_id: String,
    /// Stored priority; a higher value wins.
    pub priority: i64,
    /// Whether this profile considers the package when it is active.
    pub enabled: bool,
    /// Claimed files of this package that another member wins.
    pub shadowed_files: usize,
}

/// Where [`set_priority`] places one package in a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Placement {
    /// Place the package immediately above this package (higher priority).
    Before(String),
    /// Place the package immediately below this package (lower priority).
    After(String),
    /// Place the package at this 1-based rank, where rank 1 is the winner.
    Rank(i64),
}

/// List every profile of `game`, active first, with memberships in priority
/// order.
pub fn list(store: &Store, game: &Path) -> Result<Vec<Profile>> {
    let game_text = game.to_string_lossy();
    let mut statement = store
        .connection
        .prepare("SELECT id FROM profiles WHERE game_path=?1 ORDER BY active DESC, name, id")?;
    let ids = statement
        .query_map([game_text.as_ref()], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ids.into_iter()
        .map(|id| read_profile(&store.connection, &id))
        .collect()
}

/// Create an inactive profile named `name`.
///
/// With `clone_from`, the new profile copies the source profile's memberships,
/// priorities and enabled flags. An existing name fails with `PROFILE_EXISTS`;
/// an unknown clone source fails with `PROFILE_NOT_FOUND`.
pub fn create(
    store: &mut Store,
    game: &Path,
    name: &str,
    clone_from: Option<&str>,
) -> Result<Profile> {
    if name.trim().is_empty() {
        return Err(coded(
            ErrorCode::InvalidArguments,
            "profile name must not be empty",
        ));
    }
    let _guard = lock_store(store)?;
    let tx = store
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let game_text = game.to_string_lossy();
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM profiles WHERE game_path=?1 AND name=?2)",
        params![game_text.as_ref(), name],
        |row| row.get(0),
    )?;
    if exists {
        return Err(coded(
            ErrorCode::ProfileExists,
            format!("profile {name:?} already exists for {}", game.display()),
        ));
    }
    let source = match clone_from {
        Some(source) => Some(profile_id_by_name(&tx, &game_text, source)?),
        None => None,
    };
    let id = crate::state::profiles::profile_id(&game_text, name);
    tx.execute(
        "INSERT INTO profiles(id,game_path,name,active) VALUES (?1,?2,?3,0)",
        params![id, game_text.as_ref(), name],
    )?;
    if let Some(source) = source {
        tx.execute(
            "INSERT INTO profile_packages(profile_id,package_id,priority,enabled)
             SELECT ?1,package_id,priority,enabled
             FROM profile_packages WHERE profile_id=?2",
            params![id, source],
        )?;
    }
    let profile = read_profile(&tx, &id)?;
    tx.commit()?;
    Ok(profile)
}

/// Delete an inactive profile.
///
/// The active profile fails with `PROFILE_ACTIVE`, and the only profile of the
/// installation fails with `PROFILE_LAST`, before any write. A package that
/// no longer belongs to a profile is removed with its claims and release
/// records; a package that another profile still contains stays.
pub fn delete(store: &mut Store, game: &Path, name: &str) -> Result<()> {
    let _guard = lock_store(store)?;
    let tx = store
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let game_text = game.to_string_lossy();
    let (id, active) = profile_row_by_name(&tx, &game_text, name)?;
    if active {
        return Err(coded(
            ErrorCode::ProfileActive,
            format!("profile {name:?} is active; switch profiles before deleting it"),
        ));
    }
    let count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM profiles WHERE game_path=?1",
        [game_text.as_ref()],
        |row| row.get(0),
    )?;
    if count <= 1 {
        bail!(
            "PROFILE_LAST: profile {name:?} is the only profile for {}; it cannot be deleted",
            game.display()
        );
    }
    let packages = {
        let mut statement =
            tx.prepare("SELECT package_id FROM profile_packages WHERE profile_id=?1")?;
        let rows = statement
            .query_map([&id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    tx.execute("DELETE FROM profile_packages WHERE profile_id=?1", [&id])?;
    tx.execute("DELETE FROM profiles WHERE id=?1", [&id])?;
    for package_id in packages {
        let remains: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM profile_packages WHERE package_id=?1)",
            [&package_id],
            |row| row.get(0),
        )?;
        if remains {
            continue;
        }
        crate::state::profiles::delete_package_records(&tx, &package_id)?;
        tx.execute("DELETE FROM managed_packages WHERE id=?1", [&package_id])?;
    }
    tx.commit()?;
    Ok(())
}

/// Move `package` in `profile` and renumber every priority contiguously.
pub fn set_priority(
    store: &mut Store,
    game: &Path,
    profile: &str,
    package: &str,
    placement: Placement,
) -> Result<()> {
    let _guard = lock_store(store)?;
    let tx = store
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let game_text = game.to_string_lossy();
    let profile_id = profile_id_by_name(&tx, &game_text, profile)?;
    match &placement {
        Placement::Before(other) | Placement::After(other) if other == package => {
            return Ok(());
        }
        _ => {}
    }
    let mut members: Vec<String> = {
        let mut statement = tx.prepare(
            "SELECT package_id FROM profile_packages
             WHERE profile_id=?1 ORDER BY priority DESC",
        )?;
        let rows = statement
            .query_map([&profile_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let count = members.len();
    let from = members
        .iter()
        .position(|member| member == package)
        .ok_or_else(|| package_not_in_profile(package, profile))?;
    let moving = members.remove(from);
    let target = match &placement {
        Placement::Rank(rank) => {
            if *rank < 1 || usize::try_from(*rank).map_or(true, |rank| rank > count) {
                return Err(coded(
                    ErrorCode::InvalidArguments,
                    format!("rank {rank} is outside 1..={count} for profile {profile:?}"),
                ));
            }
            usize::try_from(*rank).unwrap_or(count) - 1
        }
        Placement::Before(other) => members
            .iter()
            .position(|member| member == other)
            .ok_or_else(|| package_not_in_profile(other, profile))?,
        Placement::After(other) => {
            members
                .iter()
                .position(|member| member == other)
                .ok_or_else(|| package_not_in_profile(other, profile))?
                + 1
        }
    };
    members.insert(target.min(members.len()), moving);
    tx.execute(
        "UPDATE profile_packages SET priority=priority+1000000 WHERE profile_id=?1",
        [&profile_id],
    )?;
    let last = i64::try_from(members.len()).context("profile has too many packages")? - 1;
    for (index, member) in members.iter().enumerate() {
        let index = i64::try_from(index).context("profile has too many packages")?;
        let updated = tx.execute(
            "UPDATE profile_packages SET priority=?1
             WHERE profile_id=?2 AND package_id=?3",
            params![last - index, profile_id, member],
        )?;
        if updated != 1 {
            bail!("profile membership changed while reordering; retry");
        }
    }
    tx.commit()?;
    Ok(())
}

/// Enable or disable one membership of `profile`.
///
/// A disabled package keeps its priority and memberships; only the active
/// profile's disk effect changes.
pub fn set_enabled(
    store: &mut Store,
    game: &Path,
    profile: &str,
    package: &str,
    enabled: bool,
) -> Result<()> {
    let _guard = lock_store(store)?;
    let tx = store
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let game_text = game.to_string_lossy();
    let profile_id = profile_id_by_name(&tx, &game_text, profile)?;
    let updated = tx.execute(
        "UPDATE profile_packages SET enabled=?1
         WHERE profile_id=?2 AND package_id=?3",
        params![enabled, profile_id, package],
    )?;
    if updated != 1 {
        return Err(package_not_in_profile(package, profile));
    }
    tx.commit()?;
    Ok(())
}

fn profile_id_by_name(connection: &Connection, game_path: &str, name: &str) -> Result<String> {
    Ok(profile_row_by_name(connection, game_path, name)?.0)
}

fn profile_row_by_name(
    connection: &Connection,
    game_path: &str,
    name: &str,
) -> Result<(String, bool)> {
    connection
        .query_row(
            "SELECT id, active FROM profiles WHERE game_path=?1 AND name=?2",
            params![game_path, name],
            |row| Ok((row.get(0)?, row.get::<_, i64>(1)? != 0)),
        )
        .optional()?
        .ok_or_else(|| {
            coded(
                ErrorCode::ProfileNotFound,
                format!("profile {name:?} does not exist for {game_path}"),
            )
        })
}

fn read_profile(connection: &Connection, id: &str) -> Result<Profile> {
    let (id, name, created_at, active) = connection
        .query_row(
            "SELECT id, name, created_at, active FROM profiles WHERE id=?1",
            [id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get::<_, i64>(3)? != 0,
                ))
            },
        )
        .optional()?
        .context("profile disappeared while it was read")?;
    let packages = {
        let mut statement = connection.prepare(
            "SELECT member.package_id, member.priority, member.enabled,
                    (SELECT COUNT(*) FROM file_claims claim
                     WHERE claim.package_id = member.package_id
                       AND EXISTS (
                         SELECT 1 FROM file_claims winner
                         JOIN profile_packages other ON other.package_id = winner.package_id
                         WHERE other.profile_id = member.profile_id
                           AND other.enabled = 1
                           AND winner.game_path = claim.game_path
                           AND winner.root = claim.root
                           AND winner.path_key = claim.path_key
                           AND other.priority > member.priority
                       ))
             FROM profile_packages member
             WHERE member.profile_id=?1
             ORDER BY member.priority DESC",
        )?;
        let rows = statement
            .query_map([&id], |row| {
                Ok(ProfilePackage {
                    package_id: row.get(0)?,
                    priority: row.get(1)?,
                    enabled: row.get::<_, i64>(2)? != 0,
                    shadowed_files: row.get::<_, i64>(3)? as usize,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    Ok(Profile {
        id,
        name,
        created_at,
        active,
        packages,
    })
}

fn package_not_in_profile(package: &str, profile: &str) -> anyhow::Error {
    coded(
        ErrorCode::PackageNotFound,
        format!("package {package:?} is not in profile {profile:?}"),
    )
}

#[cfg(test)]
mod tests;
