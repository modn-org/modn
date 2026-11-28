//! The refusals that a switch checks before it plans any file change.

use super::super::journal::Roots;
use super::super::profile_toggle_plan::require_current;
use crate::error::{code_of, coded, ErrorCode};
use crate::state::{recorded_root, RootKind, Store};
use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use std::path::{Path, PathBuf};

/// The identity of one profile of an installation.
pub(crate) struct ProfileRef {
    pub(crate) id: String,
    pub(crate) name: String,
}

/// The facts every later phase of a switch reuses.
pub(super) struct SwitchContext {
    pub(super) game: PathBuf,
    pub(super) from: ProfileRef,
    pub(super) to: ProfileRef,
    pub(super) roots: Roots,
}

/// Run the refusal checks that need no file diff.
///
/// Checks, in order: no pending transaction or stack, the target profile
/// exists and is not active, and every managed file of the active profile is
/// current on disk.
pub(super) fn switchable(store: &Store, game: &Path, target: &str) -> Result<SwitchContext> {
    idle(store)?;
    let game = store.transaction_root(game)?;
    let game_text = game.to_string_lossy();
    let target_row: Option<(String, String, i64)> = store
        .connection
        .query_row(
            "SELECT id, name, active FROM profiles WHERE game_path=?1 AND name=?2",
            params![game_text.as_ref(), target],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((to_id, to_name, active)) = target_row else {
        return Err(coded(
            ErrorCode::ProfileNotFound,
            format!(
                "PROFILE_NOT_FOUND: profile {target:?} does not exist for {}",
                game.display()
            ),
        ));
    };
    if active != 0 {
        return Err(coded(
            ErrorCode::ProfileActive,
            format!(
                "PROFILE_ACTIVE: profile {to_name:?} is already active for {}",
                game.display()
            ),
        ));
    }
    let from_row: Option<(String, String)> = store
        .connection
        .query_row(
            "SELECT id, name FROM profiles WHERE game_path=?1 AND active=1",
            [game_text.as_ref()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((from_id, from_name)) = from_row else {
        return Err(coded(
            ErrorCode::ProfileNotFound,
            format!(
                "PROFILE_NOT_FOUND: {} has no active profile",
                game.display()
            ),
        ));
    };
    let roots = Roots {
        game: game.clone(),
        content: recorded_root(&store.connection, &game, RootKind::Content)?,
    };
    require_no_drift(store, game_text.as_ref(), &from_id, &roots)?;
    Ok(SwitchContext {
        game,
        from: ProfileRef {
            id: from_id,
            name: from_name,
        },
        to: ProfileRef {
            id: to_id,
            name: to_name,
        },
        roots,
    })
}

fn idle(store: &Store) -> Result<()> {
    store.ensure_idle().map_err(|error| match code_of(&error) {
        Some(ErrorCode::PendingTransaction) => coded(
            ErrorCode::PendingTransaction,
            format!("PENDING_TRANSACTION: {error:#}"),
        ),
        _ => error,
    })
}

fn require_no_drift(store: &Store, game_path: &str, profile_id: &str, roots: &Roots) -> Result<()> {
    let rows = {
        let mut statement = store.connection.prepare(
            "SELECT root, relative_path, blake3 FROM managed_files
             WHERE game_path=?1 AND profile_id=?2
             ORDER BY root, path_key",
        )?;
        let rows = statement
            .query_map(params![game_path, profile_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for (root, relative_path, blake3) in rows {
        let root = RootKind::from_label(&root).unwrap_or(RootKind::Game);
        let target = roots.target(root, &relative_path)?;
        require_current(&target, &blake3)?;
    }
    Ok(())
}
