use crate::transactions::{ownership_key, validate_transaction_root};
use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

/// The manager state schema version this build writes.
pub(crate) const SCHEMA_VERSION: i64 = 6;

/// Identity of one root within its installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RootKind {
    Game,
    Content,
}

impl RootKind {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Game => "game",
            Self::Content => "content",
        }
    }

    pub(crate) fn from_label(label: &str) -> Option<Self> {
        match label {
            "game" => Some(Self::Game),
            "content" => Some(Self::Content),
            _ => None,
        }
    }
}

/// Record `path` as the `kind` root of `installation`.
///
/// Refuses a canonical root that another installation or root already
/// claims, and a root whose recorded path differs.
pub(crate) fn record_root(
    connection: &Connection,
    installation: &Path,
    kind: RootKind,
    path: &Path,
) -> Result<()> {
    check_root(connection, installation, kind, path)?;
    let installation_text = installation.to_string_lossy();
    let path_text = path.to_string_lossy();
    let key = ownership_key(&path_text);
    connection.execute(
        "INSERT OR IGNORE INTO installation_roots(game_path, root, path, path_key)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            installation_text.as_ref(),
            kind.label(),
            path_text.as_ref(),
            key
        ],
    )?;
    let recorded: String = connection.query_row(
        "SELECT path FROM installation_roots WHERE game_path=?1 AND root=?2",
        params![installation_text.as_ref(), kind.label()],
        |row| row.get(0),
    )?;
    if recorded != path_text {
        bail!(
            "installation {installation_text} already records {} root {recorded}",
            kind.label()
        );
    }
    Ok(())
}

/// Refuse what [`record_root`] refuses, without writing manager state.
pub(crate) fn check_root(
    connection: &Connection,
    installation: &Path,
    kind: RootKind,
    path: &Path,
) -> Result<()> {
    let installation_text = installation.to_string_lossy();
    let path_text = path.to_string_lossy();
    let claimant: Option<(String, String)> = connection
        .query_row(
            "SELECT game_path, root FROM installation_roots WHERE path_key=?1",
            [ownership_key(&path_text)],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((game_path, root)) = claimant {
        if game_path != installation_text || root != kind.label() {
            bail!(
                "root {path_text} is already the {root} root of installation {game_path}; \
                 two installations cannot share a root"
            );
        }
    }
    let recorded: Option<String> = connection
        .query_row(
            "SELECT path FROM installation_roots WHERE game_path=?1 AND root=?2",
            params![installation_text.as_ref(), kind.label()],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(recorded) = recorded.filter(|recorded| *recorded != path_text) {
        bail!(
            "installation {installation_text} already records {} root {recorded}",
            kind.label()
        );
    }
    Ok(())
}

/// Read the recorded path of `kind` without requiring it to exist.
pub(crate) fn recorded_root(
    connection: &Connection,
    installation: &Path,
    kind: RootKind,
) -> Result<Option<PathBuf>> {
    Ok(connection
        .query_row(
            "SELECT path FROM installation_roots WHERE game_path=?1 AND root=?2",
            params![installation.to_string_lossy().as_ref(), kind.label()],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(PathBuf::from))
}

/// Whether `installation` names an installation with recorded roots.
pub(crate) fn is_recorded(connection: &Connection, installation: &Path) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM installation_roots WHERE game_path=?1)",
        [installation.to_string_lossy().as_ref()],
        |row| row.get(0),
    )?)
}

/// Resolve the `kind` root of `installation` through its recorded identity.
///
/// Fails closed when the recorded root is missing, is a link or junction, or
/// no longer canonicalizes to the recorded path. An installation without a
/// recorded game root has committed no ownership yet, so its game root is the
/// canonical form of `installation`.
pub(crate) fn resolve_root(
    connection: &Connection,
    installation: &Path,
    kind: RootKind,
) -> Result<PathBuf> {
    let recorded: Option<String> = connection
        .query_row(
            "SELECT path FROM installation_roots WHERE game_path=?1 AND root=?2",
            params![installation.to_string_lossy().as_ref(), kind.label()],
            |row| row.get(0),
        )
        .optional()?;
    let path = match (recorded, kind) {
        (Some(path), _) => PathBuf::from(path),
        (None, RootKind::Game) => return validate_transaction_root(installation),
        (None, RootKind::Content) => {
            bail!("installation {installation:?} has no recorded content root")
        }
    };
    let canonical = validate_transaction_root(&path)?;
    if canonical != path {
        bail!(
            "recorded {} root {} now resolves to {}; it moved or a parent became a link, \
             so Modn refuses to change it",
            kind.label(),
            path.display(),
            canonical.display()
        );
    }
    Ok(path)
}

#[cfg(test)]
#[path = "roots_tests.rs"]
pub(super) mod tests;
