//! Enable or disable one package membership of a profile.
//!
//! Disabling an enabled package of the active profile gives each path it wins
//! to the next enabled claimant, or removes the file when nobody claims it.
//! Enabling a disabled package puts its bytes on every path where it becomes
//! the highest enabled claimant. Every plan is checked before disk changes,
//! and the enabled flag and winner rows commit in one SQLite transaction.

use super::journal::{JournalFile, Roots};
use super::restore::RestoreCandidate;
use crate::error::{coded, ErrorCode};
use crate::state::{recorded_root, RootKind, Store};
use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use std::path::{Path, PathBuf};

/// What a profile toggle does to one game file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToggleAction {
    /// Remove the file because no enabled package claims the path.
    Remove,
    /// Restore the next enabled claimant's verified bytes.
    Restore,
    /// Replace the current winner's file with the toggled package's bytes.
    Replace,
    /// Create the toggled package's file where no managed winner exists.
    Create,
}

impl ToggleAction {
    /// The lowercase name used by `--json`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Remove => "remove",
            Self::Restore => "restore",
            Self::Replace => "replace",
            Self::Create => "create",
        }
    }
}

/// One game file a profile toggle changes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ToggleChange {
    /// Game-relative destination path.
    pub path: String,
    /// What happens to the file.
    pub action: ToggleAction,
    /// Package that supplied the bytes before the toggle.
    pub from: Option<String>,
    /// Package that supplies the bytes after the toggle.
    pub to: Option<String>,
}

/// The stored state and file changes of one profile enable or disable.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProfileToggle {
    /// User-facing profile name.
    pub profile: String,
    /// Managed package whose membership changes.
    pub package: String,
    /// Enabled flag after a successful toggle.
    pub enabled: bool,
    /// Whether this profile decides the files on disk.
    pub active: bool,
    /// File changes in execution order.
    pub changes: Vec<ToggleChange>,
}

/// A checked toggle with the bytes it needs and the roots it writes.
pub(super) struct PreparedToggle {
    pub(super) toggle: ProfileToggle,
    pub(super) profile_id: String,
    pub(super) current_enabled: bool,
    pub(super) roots: Roots,
    pub(super) entries: Vec<ToggleEntry>,
}

/// One checked file change of a prepared toggle.
pub(super) struct ToggleEntry {
    pub(super) change: ToggleChange,
    pub(super) file: JournalFile,
    pub(super) new_bytes: Option<RestoreCandidate>,
    pub(super) archive: Option<PathBuf>,
}

impl Store {
    /// Check a profile toggle and return the file changes it would make.
    ///
    /// This reads manager state and archive digests only. A missing archive or
    /// a drifted winner refuses before anything is written.
    pub fn plan_profile_toggle(
        &self,
        game: &Path,
        profile: &str,
        package: &str,
        enabled: bool,
    ) -> Result<ProfileToggle> {
        Ok(self
            .prepare_profile_toggle(game, profile, package, enabled)?
            .toggle)
    }

    /// Enable or disable `package` in `profile`, changing game files when the
    /// profile is active and the package wins or reclaims a path.
    pub fn profile_toggle(
        &mut self,
        game: &Path,
        profile: &str,
        package: &str,
        enabled: bool,
    ) -> Result<ProfileToggle> {
        let _guard = super::lock_store(self)?;
        self.ensure_no_pending()?;
        self.profile_toggle_locked(game, profile, package, enabled)
    }

    pub(super) fn profile_toggle_locked(
        &mut self,
        game: &Path,
        profile: &str,
        package: &str,
        enabled: bool,
    ) -> Result<ProfileToggle> {
        self.profile_toggle_with_fault_locked(game, profile, package, enabled, None)
    }

    pub(super) fn profile_toggle_with_fault_locked(
        &mut self,
        game: &Path,
        profile: &str,
        package: &str,
        enabled: bool,
        fault: Option<super::install::Fault>,
    ) -> Result<ProfileToggle> {
        let prepared = self.prepare_profile_toggle(game, profile, package, enabled)?;
        if prepared.entries.is_empty() {
            self.record_toggle_state(&prepared)?;
            return Ok(prepared.toggle);
        }
        super::profile_toggle_apply::execute_toggle(self, prepared, fault)
    }

    fn prepare_profile_toggle(
        &self,
        game: &Path,
        profile: &str,
        package: &str,
        enabled: bool,
    ) -> Result<PreparedToggle> {
        let game = self.transaction_root(game)?;
        let game_text = game.to_string_lossy();
        let row = self
            .connection
            .query_row(
                "SELECT profile.id, profile.active, member.enabled, member.priority
                 FROM profiles profile
                 JOIN profile_packages member ON member.profile_id = profile.id
                 WHERE profile.game_path=?1 AND profile.name=?2 AND member.package_id=?3",
                params![game_text.as_ref(), profile, package],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)? != 0,
                        row.get::<_, i64>(2)? != 0,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((profile_id, active, current_enabled, priority)) = row else {
            return Err(membership_not_found(self, &game_text, profile, package));
        };
        let toggle = ProfileToggle {
            profile: profile.to_owned(),
            package: package.to_owned(),
            enabled,
            active,
            changes: Vec::new(),
        };
        if current_enabled == enabled || !active {
            return Ok(PreparedToggle {
                toggle,
                profile_id,
                current_enabled,
                roots: Roots {
                    game,
                    content: None,
                },
                entries: Vec::new(),
            });
        }
        let mut prepared = PreparedToggle {
            toggle,
            profile_id,
            current_enabled,
            roots: Roots {
                game,
                content: None,
            },
            entries: Vec::new(),
        };
        prepared.roots.content = if self.membership_has_content(&prepared)? {
            recorded_root(&self.connection, &prepared.roots.game, RootKind::Content)?
        } else {
            None
        };
        prepared.entries = if enabled {
            self.plan_enable(&prepared, package, priority)?
        } else {
            self.plan_disable(&prepared, package)?
        };
        prepared.toggle.changes = prepared
            .entries
            .iter()
            .map(|entry| entry.change.clone())
            .collect();
        Ok(prepared)
    }

    fn membership_has_content(&self, prepared: &PreparedToggle) -> Result<bool> {
        let game_text = prepared.roots.game.to_string_lossy();
        Ok(self.connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM managed_files file
                 JOIN profile_packages member ON member.package_id = file.package_id
                 WHERE file.game_path=?1 AND member.profile_id=?2 AND file.root=?3
                 UNION
                 SELECT 1 FROM file_claims claim
                 JOIN profile_packages member ON member.package_id = claim.package_id
                 WHERE claim.game_path=?1 AND member.profile_id=?2 AND claim.root=?3)",
            params![
                game_text.as_ref(),
                prepared.profile_id,
                RootKind::Content.label()
            ],
            |row| row.get(0),
        )?)
    }

    fn record_toggle_state(&mut self, prepared: &PreparedToggle) -> Result<()> {
        if prepared.current_enabled == prepared.toggle.enabled {
            return Ok(());
        }
        let updated = self.connection.execute(
            "UPDATE profile_packages SET enabled=?1 WHERE profile_id=?2 AND package_id=?3",
            params![
                prepared.toggle.enabled,
                prepared.profile_id,
                prepared.toggle.package
            ],
        )?;
        if updated != 1 {
            anyhow::bail!("profile membership changed while the toggle ran; retry");
        }
        Ok(())
    }
}

fn membership_not_found(store: &Store, game: &str, profile: &str, package: &str) -> anyhow::Error {
    let exists: bool = store
        .connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM profiles WHERE game_path=?1 AND name=?2)",
            params![game, profile],
            |row| row.get(0),
        )
        .unwrap_or(false);
    if exists {
        coded(
            ErrorCode::PackageNotFound,
            format!("package {package:?} is not in profile {profile:?}"),
        )
    } else {
        coded(
            ErrorCode::ProfileNotFound,
            format!("profile {profile:?} does not exist for {game}"),
        )
    }
}

/// The active profile identifier for `game`.
pub(super) fn active_profile_id(store: &Store, game: &Path) -> Result<String> {
    Ok(store.connection.query_row(
        "SELECT id FROM profiles WHERE game_path=?1 AND active=1",
        [game.to_string_lossy().as_ref()],
        |row| row.get(0),
    )?)
}

/// One pending file row of a profile toggle journal.
pub(super) struct CommitFile {
    pub(super) ordinal: usize,
    pub(super) root: String,
    pub(super) path_key: String,
    pub(super) relative_path: String,
    pub(super) game_path: String,
}

impl Store {
    /// The journal files of `id`, in ordinal order.
    pub(super) fn toggle_journal_files(&self, id: &str) -> Result<Vec<CommitFile>> {
        let mut statement = self.connection.prepare(
            "SELECT file.ord, file.root, file.relative_path, journal.game_path
             FROM pending_files file
             JOIN pending_transactions journal ON journal.id = file.transaction_id
             WHERE file.transaction_id=?1 ORDER BY file.ord",
        )?;
        let rows = statement
            .query_map([id], |row| {
                let destination: String = row.get(2)?;
                Ok(CommitFile {
                    ordinal: row.get::<_, i64>(0)? as usize,
                    root: row.get(1)?,
                    path_key: super::ownership_key(&destination),
                    relative_path: destination,
                    game_path: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}
