//! Durable per-mod approval of native-injector and executable capabilities.

use crate::error::{coded, ErrorCode};
use crate::state::Store;
use crate::transactions::{ownership_key, CatalogReference};
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::Path;

/// Manager state schema version that adds the approval table.
pub(crate) const SCHEMA_VERSION: i64 = 5;

/// Capabilities of one mod release that need explicit user approval.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Capabilities {
    /// Whether the release injects native code into the game process.
    pub native_injector: bool,
    /// Normalized game-relative executable payload paths.
    pub executables: BTreeSet<String>,
}

/// Capabilities that neither a stored approval nor an explicit flag covers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Uncovered {
    /// Whether the release needs a newly approved native injector.
    pub native_injector: bool,
    /// Executable payload paths that need approval.
    pub executables: Vec<String>,
}

impl Capabilities {
    /// Build a capability set, normalizing each executable path.
    #[must_use]
    pub fn new(
        native_injector: bool,
        executables: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Self {
        Self {
            native_injector,
            executables: executables
                .into_iter()
                .map(|path| normalize(path.as_ref()))
                .collect(),
        }
    }

    /// Whether the set records no capability.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.native_injector && self.executables.is_empty()
    }

    /// Whether this approval covers every capability in `needed`.
    #[must_use]
    pub fn covers(&self, needed: &Self) -> bool {
        (!needed.native_injector || self.native_injector)
            && needed.executables.iter().all(|path| self.contains(path))
    }

    /// The subset of `self` that `approved` covers, keeping this set's spelling.
    #[must_use]
    pub fn covered(&self, approved: &Self) -> Self {
        Self {
            native_injector: self.native_injector && approved.native_injector,
            executables: self
                .executables
                .iter()
                .filter(|path| approved.contains(path))
                .cloned()
                .collect(),
        }
    }

    /// The capabilities of `self` that `approved` does not cover.
    #[must_use]
    pub fn uncovered_by(&self, approved: &Self) -> Uncovered {
        Uncovered {
            native_injector: self.native_injector && !approved.native_injector,
            executables: self
                .executables
                .iter()
                .filter(|path| !approved.contains(path))
                .cloned()
                .collect(),
        }
    }

    /// Both capability sets, without duplicate executable paths.
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        let mut executables = self.executables.clone();
        for path in &other.executables {
            if !executables.iter().any(|existing| same_path(existing, path)) {
                executables.insert(path.clone());
            }
        }
        Self {
            native_injector: self.native_injector || other.native_injector,
            executables,
        }
    }

    fn contains(&self, path: &str) -> bool {
        self.executables
            .iter()
            .any(|existing| same_path(existing, path))
    }
}

impl Uncovered {
    /// An uncovered native injector with no executable payloads.
    #[must_use]
    pub fn native_injector() -> Self {
        Self {
            native_injector: true,
            executables: Vec::new(),
        }
    }

    /// Whether every capability is covered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.native_injector && self.executables.is_empty()
    }
}

/// Refusal for a release that needs capabilities the caller did not approve.
#[derive(Debug)]
pub struct ApprovalRefusal {
    reference: CatalogReference,
    uncovered: Uncovered,
}

impl ApprovalRefusal {
    /// Refuse `reference` with the capabilities that remain uncovered.
    #[must_use]
    pub fn new(reference: CatalogReference, uncovered: Uncovered) -> Self {
        Self {
            reference,
            uncovered,
        }
    }

    /// The stable code for this refusal.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        if self.uncovered.native_injector {
            ErrorCode::NativeInjectorNotAllowed
        } else {
            ErrorCode::ApprovalRequired
        }
    }

    /// Structured error envelope for `--json` output.
    #[must_use]
    pub fn as_json(&self) -> serde_json::Value {
        let mut flags = Vec::new();
        if self.uncovered.native_injector {
            flags.push("--allow-native-injector");
        }
        if !self.uncovered.executables.is_empty() {
            flags.push("--allow-executable");
        }
        let mut value = serde_json::json!({
            "error": self.code().as_str(),
            "mod_id": self.reference.mod_id,
            "package": self.reference.to_string(),
            "uncovered": self.uncovered,
            "flags": flags,
        });
        if let [flag] = flags.as_slice() {
            value["flag"] = serde_json::Value::String((*flag).to_owned());
        }
        value
    }

    /// Convert into an error that keeps both the stable code and this envelope.
    #[must_use]
    pub fn into_error(self) -> anyhow::Error {
        let message = self.to_string();
        let code = self.code();
        anyhow::Error::new(self).context(coded(code, message))
    }
}

impl std::fmt::Display for ApprovalRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let package = &self.reference;
        if self.uncovered.native_injector && self.uncovered.executables.is_empty() {
            return write!(
                f,
                "{package} is a native injector; pass --allow-native-injector to install or update it"
            );
        }
        if self.uncovered.native_injector {
            return write!(
                f,
                "{package} needs approval for native code and executable payload(s) {}; pass \
                 --allow-native-injector and --allow-executable",
                self.uncovered.executables.join(", ")
            );
        }
        write!(
            f,
            "{package} needs approval for executable payload(s) {}; pass --allow-executable for \
             each path",
            self.uncovered.executables.join(", ")
        )
    }
}

impl std::error::Error for ApprovalRefusal {}

impl Store {
    /// Read the stored approval of one mod of one bucket in one game
    /// installation.
    pub fn approval(
        &self,
        game: &Path,
        bucket: &str,
        mod_id: &str,
    ) -> Result<Option<Capabilities>> {
        let row = self
            .connection
            .query_row(
                "SELECT native_injector, executables FROM mod_approvals
                 WHERE game_path=?1 AND bucket=?2 AND mod_id=?3",
                params![game.to_string_lossy().as_ref(), bucket, mod_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        row.map(|(native_injector, executables)| {
            let executables: Vec<String> = serde_json::from_str(&executables)
                .context("stored mod approval has invalid executable paths")?;
            Ok(Capabilities::new(native_injector != 0, executables))
        })
        .transpose()
    }

    /// Widen the stored approval of one mod of one bucket to `capabilities`.
    ///
    /// The update never shrinks a recorded native-injector or executable
    /// capability. Callers record an approval only after their operation
    /// commits.
    pub fn record_approval(
        &mut self,
        game: &Path,
        bucket: &str,
        mod_id: &str,
        capabilities: &Capabilities,
    ) -> Result<()> {
        if mod_id.is_empty() {
            bail!("mod id must not be empty");
        }
        let widened = match self.approval(game, bucket, mod_id)? {
            Some(stored) => stored.union(capabilities),
            None => capabilities.clone(),
        };
        let executables =
            serde_json::to_string(&widened.executables).context("cannot serialize approval")?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO mod_approvals(game_path, bucket, mod_id, native_injector, executables)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(game_path, bucket, mod_id) DO UPDATE SET
                 native_injector=excluded.native_injector,
                 executables=excluded.executables",
            params![
                game.to_string_lossy().as_ref(),
                bucket,
                mod_id,
                i64::from(widened.native_injector),
                executables
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
}

pub(crate) fn ensure_schema(connection: &Connection) -> Result<()> {
    let has_bucket: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('mod_approvals')
         WHERE name='bucket')",
        [],
        |row| row.get(0),
    )?;
    if !has_bucket {
        // An unreleased schema-5 build keyed approvals by mod ID only. A row
        // without a bucket cannot be attributed to one publisher, so rebuild
        // the table and ask the user again (fail closed).
        connection.execute_batch(
            "DROP TABLE IF EXISTS mod_approvals;
             CREATE TABLE mod_approvals (
                 game_path TEXT NOT NULL,
                 bucket TEXT NOT NULL,
                 mod_id TEXT NOT NULL,
                 native_injector INTEGER NOT NULL DEFAULT 0,
                 executables TEXT NOT NULL DEFAULT '[]',
                 approved_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                 PRIMARY KEY (game_path, bucket, mod_id)
             );",
        )?;
    }
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version < SCHEMA_VERSION {
        connection.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    Ok(())
}


fn normalize(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches('/').to_owned()
}

fn same_path(left: &str, right: &str) -> bool {
    ownership_key(left) == ownership_key(right)
}

#[cfg(test)]
#[path = "approval_tests.rs"]
mod tests;
