//! The history operation that one write committed.
//!
//! A write command's `--json` result names the committed operation so a caller
//! can undo exactly that record. The ID is read from history instead of being
//! derived from the package: an uninstall journal and an install stack have
//! their own transaction IDs, and a multi-package write commits one record per
//! package.

use anyhow::Result;
use modn_core::state::Store;
use std::collections::BTreeSet;

/// The operation IDs that existed before a write.
pub(super) struct KnownOperations(BTreeSet<String>);

impl KnownOperations {
    /// Snapshot the operation IDs recorded for `game_id`.
    pub(super) fn read(store: &Store, game_id: &str) -> Result<Self> {
        let ids = store
            .history(Some(game_id))?
            .into_iter()
            .map(|operation| operation.id)
            .collect();
        Ok(Self(ids))
    }

    /// The one operation committed since the snapshot, if exactly one.
    ///
    /// A write that commits several records, such as a multi-package
    /// uninstall, has no single ID, so it reports none instead of guessing.
    pub(super) fn committed(self, store: &Store, game_id: &str) -> Result<Option<String>> {
        let committed: Vec<String> = store
            .history(Some(game_id))?
            .into_iter()
            .map(|operation| operation.id)
            .filter(|id| !self.0.contains(id))
            .collect();
        Ok(match committed.as_slice() {
            [id] => Some(id.clone()),
            _ => None,
        })
    }
}
