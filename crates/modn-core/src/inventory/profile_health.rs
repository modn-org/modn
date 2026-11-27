//! Read-only health of the active profile and of the archives a restore needs.
//!
//! The active profile decides the files on disk. Its enabled packages claim
//! paths, and the enabled package with the highest priority wins each path.
//! This module compares the stored winner of every claimed path with the
//! priority winner, and checks that the verified archive cache still holds the
//! bytes needed to restore a shadowed claim or to switch to another profile.
//!
//! Every lookup is a read. Doctor never repairs, never writes game files and
//! never changes the archive cache.

use crate::acquisition::{source_cache_url, ArchiveCache, CacheKey};
use crate::history::package_metadata;
use crate::state::Store;
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

/// Health of one installation's active profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct ProfileHealth {
    /// User-facing name of the active profile.
    pub name: String,
    /// Whether the profile decides the files on disk. Always true for the
    /// active profile this report describes.
    pub active: bool,
    /// Memberships in the profile, enabled and disabled.
    pub packages: usize,
    /// Paths that at least two enabled members claim.
    pub overlapping_files: usize,
    /// True when no issue was found.
    pub consistent: bool,
    /// Issues ordered by kind, then path, then package.
    pub issues: Vec<ProfileIssue>,
}

/// One profile health issue.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[non_exhaustive]
pub struct ProfileIssue {
    /// What is wrong.
    pub kind: IssueKind,
    /// Root-relative path, prefixed with its root label when that is not
    /// `game`.
    pub path: String,
    /// Package the issue is about.
    pub package: String,
    /// Observed state, in one sentence.
    pub detail: String,
    /// Safe command that resolves the issue.
    pub advice: String,
}

/// The kind of a profile health issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum IssueKind {
    /// The stored winner of a path is not the priority winner.
    WinnerMismatch,
    /// A shadowed claim cannot be restored: its verified archive is missing or
    /// its bytes changed.
    ShadowedClaimUnrecoverable,
    /// A package that a profile switch would write has no verified archive.
    OtherProfileArchiveMissing,
}

impl IssueKind {
    /// Stable label used in text and JSON output.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::WinnerMismatch => "winner_mismatch",
            Self::ShadowedClaimUnrecoverable => "shadowed_claim_unrecoverable",
            Self::OtherProfileArchiveMissing => "other_profile_archive_missing",
        }
    }

    /// Safe command that resolves an issue of this kind.
    #[must_use]
    pub const fn advice(self) -> &'static str {
        match self {
            Self::WinnerMismatch => "modn profile apply --yes",
            Self::ShadowedClaimUnrecoverable | Self::OtherProfileArchiveMissing => {
                "modn cache show"
            }
        }
    }
}

impl fmt::Display for IssueKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Read the health of the active profile of `game`, or `None` when the
/// installation has no active profile.
///
/// # Errors
///
/// Returns a state read error when the database cannot be queried.
pub fn health(store: &Store, game: &Path) -> Result<Option<ProfileHealth>> {
    let game_text = game.to_string_lossy();
    let Some((profile_id, name)) = active_profile(&store.connection, &game_text)? else {
        return Ok(None);
    };
    let packages = member_count(&store.connection, &profile_id)?;
    let claims = claims(&store.connection, &game_text, &profile_id)?;
    let groups = groups(&claims);
    let stored = stored_winners(&store.connection, &game_text)?;
    let cache = ArchiveCache::new(cache_root(store));
    let mut archives = Archives::new(store, &cache);
    let mut issues = Vec::new();
    for group in &groups {
        let group = *group;
        let winner = &group[0];
        match stored.get(&(winner.root.clone(), winner.path_key.clone())) {
            Some(stored) if stored == &winner.package => {}
            Some(stored) => issues.push(issue(
                IssueKind::WinnerMismatch,
                winner,
                format!("the stored winner is {stored}"),
            )),
            None => issues.push(issue(
                IssueKind::WinnerMismatch,
                winner,
                "no managed file records a winner for the claimed path".into(),
            )),
        }
        for shadowed in &group[1..] {
            if let Some(detail) = archives.problem(shadowed)? {
                issues.push(issue(
                    IssueKind::ShadowedClaimUnrecoverable,
                    shadowed,
                    detail,
                ));
            }
        }
    }
    issues.extend(other_profile_issues(store, &game_text, &mut archives)?);
    issues.sort();
    Ok(Some(ProfileHealth {
        name,
        active: true,
        packages,
        overlapping_files: groups.iter().filter(|group| group.len() > 1).count(),
        consistent: issues.is_empty(),
        issues,
    }))
}

fn issue(kind: IssueKind, claim: &Claim, detail: String) -> ProfileIssue {
    ProfileIssue {
        kind,
        path: claim.display_path(),
        package: claim.package.clone(),
        detail,
        advice: kind.advice().into(),
    }
}

/// One enabled claim of a profile.
#[derive(Clone)]
struct Claim {
    root: String,
    path_key: String,
    relative_path: String,
    package: String,
    archive_entry: String,
    bytes: u64,
}

impl Claim {
    /// Root-relative path, prefixed with its root label when that is not
    /// `game`.
    fn display_path(&self) -> String {
        if self.root == "game" {
            self.relative_path.clone()
        } else {
            format!("{}/{}", self.root, self.relative_path)
        }
    }
}

fn cache_root(store: &Store) -> PathBuf {
    if let Some(root) = std::env::var_os("MODN_CACHE").filter(|value| !value.is_empty()) {
        return root.into();
    }
    store.state_root.join("cache")
}

fn active_profile(connection: &Connection, game: &str) -> Result<Option<(String, String)>> {
    Ok(connection
        .query_row(
            "SELECT id, name FROM profiles WHERE game_path=?1 AND active=1",
            [game],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?)
}

fn member_count(connection: &Connection, profile_id: &str) -> Result<usize> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM profile_packages WHERE profile_id=?1",
        [profile_id],
        |row| row.get(0),
    )?;
    Ok(count.max(0) as usize)
}

fn claims(connection: &Connection, game: &str, profile_id: &str) -> Result<Vec<Claim>> {
    let mut statement = connection.prepare(
        "SELECT claim.root, claim.path_key, claim.relative_path, claim.package_id,
                claim.archive_entry, claim.bytes
         FROM file_claims claim
         JOIN profile_packages member ON member.package_id = claim.package_id
         WHERE claim.game_path=?1 AND member.profile_id=?2 AND member.enabled=1
         ORDER BY claim.root, claim.path_key, member.priority DESC, claim.package_id",
    )?;
    let rows = statement
        .query_map(params![game, profile_id], |row| {
            Ok(Claim {
                root: row.get(0)?,
                path_key: row.get(1)?,
                relative_path: row.get(2)?,
                package: row.get(3)?,
                archive_entry: row.get(4)?,
                bytes: row.get::<_, i64>(5)? as u64,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Group ordered claims by root and ownership key; the first claim of a group
/// is the priority winner.
fn groups(claims: &[Claim]) -> Vec<&[Claim]> {
    let mut groups = Vec::new();
    let mut start = 0;
    while start < claims.len() {
        let mut end = start + 1;
        while end < claims.len()
            && claims[end].root == claims[start].root
            && claims[end].path_key == claims[start].path_key
        {
            end += 1;
        }
        groups.push(&claims[start..end]);
        start = end;
    }
    groups
}

fn stored_winners(
    connection: &Connection,
    game: &str,
) -> Result<HashMap<(String, String), String>> {
    let mut statement = connection
        .prepare("SELECT root, path_key, package_id FROM managed_files WHERE game_path=?1")?;
    let rows = statement
        .query_map([game], |row| {
            Ok((
                (row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows.into_iter().collect())
}

/// Missing archives of every package that a switch to another profile writes.
fn other_profile_issues(
    store: &Store,
    game: &str,
    archives: &mut Archives<'_>,
) -> Result<Vec<ProfileIssue>> {
    let mut statement = store.connection.prepare(
        "SELECT id, name FROM profiles WHERE game_path=?1 AND active=0 ORDER BY name, id",
    )?;
    let profiles = statement
        .query_map([game], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut issues = Vec::new();
    for (profile_id, name) in profiles {
        let claims = claims(&store.connection, game, &profile_id)?;
        for group in groups(&claims) {
            let winner = group[0].clone();
            if let Some(detail) = archives.problem(&winner)? {
                issues.push(issue(
                    IssueKind::OtherProfileArchiveMissing,
                    &winner,
                    format!("profile {name:?} would need this file: {detail}"),
                ));
            }
        }
    }
    Ok(issues)
}

/// Verified archive lookups for one report, cached per package and per archive.
struct Archives<'a> {
    store: &'a Store,
    cache: &'a ArchiveCache,
    resolved: HashMap<String, Option<PathBuf>>,
    inspected: HashMap<PathBuf, Option<Vec<(String, u64)>>>,
}

impl<'a> Archives<'a> {
    fn new(store: &'a Store, cache: &'a ArchiveCache) -> Self {
        Self {
            store,
            cache,
            resolved: HashMap::new(),
            inspected: HashMap::new(),
        }
    }

    /// Why `claim` cannot be restored, or `None` when its bytes are available.
    ///
    /// The verified cache re-hashes the whole archive before it answers, so a
    /// changed entry digest also fails the archive digest and is reported here.
    fn problem(&mut self, claim: &Claim) -> Result<Option<String>> {
        let Some(archive) = self.archive(&claim.package)? else {
            return Ok(Some(
                "the verified archive cache holds no archive with this package's recorded \
                 digest"
                    .into(),
            ));
        };
        if !self.inspected.contains_key(&archive) {
            let files = crate::archive::inspect(&archive).ok().map(|inspection| {
                inspection
                    .files
                    .into_iter()
                    .map(|file| (file.entry, file.bytes))
                    .collect::<Vec<_>>()
            });
            self.inspected.insert(archive.clone(), files);
        }
        let entries = self
            .inspected
            .get(&archive)
            .expect("the archive was just inspected");
        let matches = entries.as_ref().is_some_and(|entries| {
            entries
                .iter()
                .any(|(entry, bytes)| *entry == claim.archive_entry && *bytes == claim.bytes)
        });
        if matches {
            Ok(None)
        } else {
            Ok(Some(format!(
                "the verified archive has no entry {:?} with the recorded size",
                claim.archive_entry
            )))
        }
    }

    fn archive(&mut self, package_id: &str) -> Result<Option<PathBuf>> {
        if let Some(path) = self.resolved.get(package_id) {
            return Ok(path.clone());
        }
        let path = verified_archive(self.store, self.cache, package_id)?;
        self.resolved.insert(package_id.to_owned(), path.clone());
        Ok(path)
    }
}

/// The cache entry that holds the recorded archive of `package_id`.
///
/// This mirrors the read-only lookup of a profile restore. A catalog package
/// is looked up by its cache key; any other package is looked up by its pinned
/// archive digest. Every hit is re-hashed by the cache before it is returned.
fn verified_archive(
    store: &Store,
    cache: &ArchiveCache,
    package_id: &str,
) -> Result<Option<PathBuf>> {
    let pin: Option<String> = store
        .connection
        .query_row(
            "SELECT archive_blake3 FROM managed_packages WHERE id=?1",
            [package_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(pin) = pin else {
        return Ok(None);
    };
    let pin = pin.strip_prefix("blake3:").unwrap_or(&pin);
    if !valid_digest(pin) {
        return Ok(None);
    }
    if let Some(path) = keyed_archive(store, cache, package_id, pin)? {
        return Ok(Some(path));
    }
    for entry in cache.entries(None)? {
        if entry.retired {
            continue;
        }
        let Ok(digest) = file_digest(&entry.path) else {
            continue;
        };
        if digest == pin {
            return Ok(Some(entry.path));
        }
    }
    Ok(None)
}

fn keyed_archive(
    store: &Store,
    cache: &ArchiveCache,
    package_id: &str,
    pin: &str,
) -> Result<Option<PathBuf>> {
    let Some(metadata) = package_metadata(&store.connection, package_id)? else {
        return Ok(None);
    };
    let (Some(catalog), Some(version), Some(source)) =
        (metadata.catalog, metadata.version, metadata.source)
    else {
        return Ok(None);
    };
    let Some(url) = source_cache_url(&source)? else {
        return Ok(None);
    };
    let Ok(digest) = crate::acquisition::ArchiveDigest::new(pin) else {
        return Ok(None);
    };
    let key = CacheKey::new(
        &catalog.game_id,
        &catalog.mod_id,
        &version,
        &url,
        digest.as_str(),
    );
    cache.cached(&key)
}

fn file_digest(path: &Path) -> Result<String> {
    let mut input = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    io::copy(&mut input, &mut hasher)?;
    Ok(hasher.finalize().to_hex().to_string())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
