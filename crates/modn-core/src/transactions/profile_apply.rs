//! Applying a profile's priority order to the game files.
//!
//! Rank 1 is the winner. For every path that an enabled package of the active
//! profile claims, the expected winner is the highest-priority claimant. A
//! plan compares that winner with the stored winner row and checks the current
//! bytes, the verified archive cache and the claim digest before anything is
//! written. Execution stages every needed archive member first, records a
//! `profile_apply` journal, and commits the winner rows (and, for
//! [`Store::set_priority_applied`], the new ranks) in one SQLite transaction.
//! A crash is recovered by `modn recover --yes`, which reuses the profile
//! toggle recovery path.

use super::profile_apply_plan::PreparedApply;
use super::profile_toggle::ToggleChange;
use super::profile_toggle_apply::{self, write_ranks};
use crate::error::{coded, ErrorCode};
use crate::profiles::Placement;
use crate::state::Store;
use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use std::path::Path;

/// The stored state and file changes of one profile apply.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProfileApply {
    /// User-facing profile name.
    pub profile: String,
    /// Whether this profile decides the files on disk.
    pub active: bool,
    /// File changes in execution order.
    pub changes: Vec<ToggleChange>,
}

/// The result of one `profile priority` run.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PriorityReport {
    /// User-facing profile name.
    pub profile: String,
    /// Managed package that moved.
    pub package: String,
    /// One-based rank after the run.
    pub rank: usize,
    /// Whether the run also applied the new order to the game files.
    pub applied: bool,
    /// File changes in execution order.
    pub changes: Vec<ToggleChange>,
    /// Whether the run was a dry run.
    pub dry_run: bool,
}

impl Store {
    /// Plan the active profile's priority order without writing anything.
    pub fn plan_profile_apply(&self, game: &Path, profile: Option<&str>) -> Result<ProfileApply> {
        Ok(self.prepare_apply(game, profile, None)?.report)
    }

    /// Make the game files of `profile` match its priority order.
    pub fn apply_profile(&mut self, game: &Path, profile: Option<&str>) -> Result<ProfileApply> {
        let _guard = super::lock_store(self)?;
        self.ensure_no_pending()?;
        self.apply_profile_locked(game, profile)
    }

    fn apply_profile_locked(&mut self, game: &Path, profile: Option<&str>) -> Result<ProfileApply> {
        self.apply_profile_fault_locked(game, profile, None)
    }

    fn apply_profile_fault_locked(
        &mut self,
        game: &Path,
        profile: Option<&str>,
        fault: Option<super::install::Fault>,
    ) -> Result<ProfileApply> {
        let prepared = self.prepare_apply(game, profile, None)?;
        if !prepared.report.active {
            return Err(inactive_profile(&prepared.report.profile));
        }
        if prepared.prepared.entries.is_empty() {
            return Ok(prepared.report);
        }
        self.execute_apply(prepared, None, fault)
    }

    /// Move one package and change only the stored ranks.
    pub fn move_priority(
        &mut self,
        game: &Path,
        profile: &str,
        package: &str,
        placement: &Placement,
    ) -> Result<PriorityReport> {
        let _guard = super::lock_store(self)?;
        self.ensure_no_pending()?;
        let game = self.transaction_root(game)?;
        let (profile_id, _, members) = self.priority_members(&game, profile, package)?;
        let order = reorder_members(&members, package, placement)?;
        let rank = order
            .iter()
            .position(|member| member == package)
            .map_or(1, |index| index + 1);
        self.commit_ranks(&profile_id, &order)?;
        Ok(PriorityReport {
            profile: profile.to_owned(),
            package: package.to_owned(),
            rank,
            applied: false,
            changes: Vec::new(),
            dry_run: false,
        })
    }

    /// Move one package and optionally apply the new order in the same run.
    pub fn set_priority_applied(
        &mut self,
        game: &Path,
        profile: &str,
        package: &str,
        placement: &Placement,
        apply: bool,
    ) -> Result<PriorityReport> {
        let _guard = super::lock_store(self)?;
        self.ensure_no_pending()?;
        self.set_priority_applied_locked(game, profile, package, placement, apply, None)
    }

    #[cfg(test)]
    pub(super) fn set_priority_applied_with_fault_locked(
        &mut self,
        game: &Path,
        profile: &str,
        package: &str,
        placement: &Placement,
        apply: bool,
        fault: Option<super::install::Fault>,
    ) -> Result<PriorityReport> {
        self.set_priority_applied_locked(game, profile, package, placement, apply, fault)
    }

    fn set_priority_applied_locked(
        &mut self,
        game: &Path,
        profile: &str,
        package: &str,
        placement: &Placement,
        apply: bool,
        fault: Option<super::install::Fault>,
    ) -> Result<PriorityReport> {
        let game = self.transaction_root(game)?;
        let (profile_id, active, members) = self.priority_members(&game, profile, package)?;
        let order = reorder_members(&members, package, placement)?;
        let rank = order
            .iter()
            .position(|member| member == package)
            .map_or(1, |index| index + 1);
        let mut report = PriorityReport {
            profile: profile.to_owned(),
            package: package.to_owned(),
            rank,
            applied: false,
            changes: Vec::new(),
            dry_run: false,
        };
        if !apply {
            self.commit_ranks(&profile_id, &order)?;
            return Ok(report);
        }
        if !active {
            self.commit_ranks(&profile_id, &order)?;
            return Err(inactive_profile(profile));
        }
        let prepared = self.prepare_apply(&game, Some(profile), Some(&order))?;
        report.applied = true;
        report.changes = prepared.report.changes.clone();
        if prepared.prepared.entries.is_empty() {
            self.commit_ranks(&profile_id, &order)?;
            return Ok(report);
        }
        self.execute_apply(prepared, Some(&order), fault)?;
        Ok(report)
    }

    /// Plan one priority move, including the file changes `--apply` would make.
    pub fn plan_priority(
        &self,
        game: &Path,
        profile: &str,
        package: &str,
        placement: &Placement,
        dry_run: bool,
    ) -> Result<PriorityReport> {
        let game = self.transaction_root(game)?;
        let (_, _, members) = self.priority_members(&game, profile, package)?;
        let order = reorder_members(&members, package, placement)?;
        let rank = order
            .iter()
            .position(|member| member == package)
            .map_or(1, |index| index + 1);
        let prepared = self.prepare_apply(&game, Some(profile), Some(&order))?;
        Ok(PriorityReport {
            profile: prepared.report.profile,
            package: package.to_owned(),
            rank,
            applied: false,
            changes: prepared.report.changes,
            dry_run,
        })
    }

    fn priority_members(
        &self,
        game: &Path,
        profile: &str,
        package: &str,
    ) -> Result<(String, bool, Vec<String>)> {
        let game_text = game.to_string_lossy();
        let row: Option<(String, i64)> = self
            .connection
            .query_row(
                "SELECT id, active FROM profiles WHERE game_path=?1 AND name=?2",
                params![game_text.as_ref(), profile],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((profile_id, active)) = row else {
            return Err(coded(
                ErrorCode::ProfileNotFound,
                format!("profile {profile:?} does not exist for {}", game.display()),
            ));
        };
        let members = self.member_order(&profile_id)?;
        if !members.iter().any(|member| member == package) {
            return Err(coded(
                ErrorCode::PackageNotFound,
                format!("package {package:?} is not in profile {profile:?}"),
            ));
        }
        Ok((profile_id, active != 0, members))
    }

    fn member_order(&self, profile_id: &str) -> Result<Vec<String>> {
        let mut statement = self.connection.prepare(
            "SELECT package_id FROM profile_packages
             WHERE profile_id=?1 ORDER BY priority DESC",
        )?;
        let members = statement
            .query_map([profile_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(members)
    }

    fn commit_ranks(&mut self, profile_id: &str, order: &[String]) -> Result<()> {
        let tx = self.connection.transaction()?;
        write_ranks(&tx, profile_id, order)?;
        tx.commit()?;
        Ok(())
    }

    fn execute_apply(
        &mut self,
        prepared: PreparedApply,
        ranks: Option<&[String]>,
        fault: Option<super::install::Fault>,
    ) -> Result<ProfileApply> {
        let profile_id = prepared.profile_id;
        let report = prepared.report;
        let ranks = ranks.map(<[String]>::to_vec);
        profile_toggle_apply::execute_prepared(
            self,
            &prepared.prepared,
            "",
            fault,
            move |store, id| {
                profile_toggle_apply::commit_apply(store, id, &profile_id, ranks.as_deref())
            },
        )?;
        Ok(report)
    }
}

/// Return `members` with `package` moved to `placement`; rank 1 is the winner.
pub fn reorder_members(
    members: &[String],
    package: &str,
    placement: &Placement,
) -> Result<Vec<String>> {
    if let Placement::Before(other) | Placement::After(other) = placement {
        if other == package {
            return Ok(members.to_vec());
        }
    }
    let mut members = members.to_vec();
    let from = members
        .iter()
        .position(|member| member == package)
        .ok_or_else(|| {
            coded(
                ErrorCode::PackageNotFound,
                format!("package {package:?} is not in the profile"),
            )
        })?;
    members.remove(from);
    let count = members.len() + 1;
    let target = match placement {
        Placement::Rank(rank) => {
            let rank = usize::try_from(*rank).unwrap_or(0);
            if rank == 0 || rank > count {
                return Err(coded(
                    ErrorCode::InvalidArguments,
                    format!("rank {rank} is outside 1..={count}"),
                ));
            }
            rank - 1
        }
        Placement::Before(other) => members
            .iter()
            .position(|member| member == other)
            .ok_or_else(|| package_not_in_profile(other))?,
        Placement::After(other) => {
            members
                .iter()
                .position(|member| member == other)
                .ok_or_else(|| package_not_in_profile(other))?
                + 1
        }
    };
    members.insert(target.min(members.len()), package.to_owned());
    Ok(members)
}

fn package_not_in_profile(package: &str) -> anyhow::Error {
    coded(
        ErrorCode::PackageNotFound,
        format!("package {package:?} is not in the profile"),
    )
}

fn inactive_profile(profile: &str) -> anyhow::Error {
    coded(
        ErrorCode::ProfileFailed,
        format!(
            "PROFILE_FAILED: profile {profile:?} is not active; its ranks change, but game files still follow the active profile"
        ),
    )
}
