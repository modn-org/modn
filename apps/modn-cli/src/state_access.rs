//! Validate manager-state location without assuming any default game.
use crate::selection::selected_installation;
use anyhow::{bail, Context, Result};
use modn_core::discovery::recognizes_installation_for;
use modn_core::state::Store;
use std::path::{Component, Path, PathBuf};

/// Manager state as it exists before a command decides how to use it.
///
/// Loading never creates a database. Registrations identify game roots that
/// the state location must stay outside before any writable open.
pub(super) struct ManagerState {
    path: PathBuf,
    existing: Option<Store>,
    // `None` defers reading registrations until the state location is validated.
    registered_roots: Option<Vec<PathBuf>>,
}

/// Game identity and root that the manager state location must not enter.
#[derive(Default)]
pub(super) struct ProtectedTarget {
    pub(super) game: Option<String>,
    pub(super) root: Option<PathBuf>,
}

impl ManagerState {
    /// Open an existing database read-only, if one exists, and read its
    /// registered game roots immediately.
    pub(super) fn load(path: &Path) -> Result<Self> {
        let mut state = Self::load_deferred(path)?;
        state.registered_roots = Some(registered_roots(state.existing())?);
        Ok(state)
    }

    /// Open an existing database read-only, if one exists, but read registered
    /// game roots only when the state location is validated.
    ///
    /// Catalog installation reads registrations only before its approved
    /// write, so selection and preview errors keep their precedence.
    pub(super) fn load_deferred(path: &Path) -> Result<Self> {
        let existing = path
            .is_file()
            .then(|| Store::open_readonly(path))
            .transpose()?;
        Ok(Self {
            path: path.to_path_buf(),
            existing,
            registered_roots: None,
        })
    }

    /// Fail with `message` unless a database already exists.
    pub(super) fn require_existing(self, message: &str) -> Result<Self> {
        if self.existing.is_none() {
            bail!("{message}");
        }
        Ok(self)
    }

    pub(super) fn existing(&self) -> Option<&Store> {
        self.existing.as_ref()
    }

    pub(super) fn into_existing(self) -> Option<Store> {
        self.existing
    }

    /// Protected target for a game-scoped command.
    ///
    /// An explicit `--path` is protected as given. Without one, the selection
    /// comes only from existing registrations, before any writable open.
    pub(super) fn target_for(
        &self,
        path: Option<&Path>,
        requested: Option<&str>,
    ) -> Result<ProtectedTarget> {
        if let Some(path) = path {
            return Ok(ProtectedTarget {
                game: requested.map(str::to_owned),
                root: Some(path.to_path_buf()),
            });
        }
        let (id, root) = selected_installation(self.existing(), None, requested)?;
        Ok(ProtectedTarget {
            game: Some(requested.map_or(id, str::to_owned)),
            root: Some(root),
        })
    }

    /// Validate the state location, then open (or create) the writable database.
    pub(super) fn open_writable(self, target: &ProtectedTarget) -> Result<Store> {
        self.validate(target)?;
        Store::open(&self.path)
    }

    /// Validate the state location, then reuse the existing read-only database.
    pub(super) fn open_preview(self, target: &ProtectedTarget) -> Result<Store> {
        self.validate(target)?;
        self.existing
            .context("no registered game database for preview")
    }

    fn validate(&self, target: &ProtectedTarget) -> Result<()> {
        let deferred;
        let roots = match &self.registered_roots {
            Some(roots) => roots,
            None => {
                deferred = registered_roots(self.existing())?;
                &deferred
            }
        };
        validate_state_path(
            &self.path,
            target.game.as_deref(),
            target.root.as_deref(),
            roots,
        )
    }
}

fn registered_roots(store: Option<&Store>) -> Result<Vec<PathBuf>> {
    Ok(store
        .map(Store::list)
        .transpose()?
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| entry.path.canonicalize().ok())
        .collect())
}

pub(super) fn database_path(state_dir: Option<&Path>) -> Result<PathBuf> {
    if let Some(dir) = state_dir {
        return Ok(dir.join("state.sqlite"));
    }
    if let Some(path) = std::env::var_os("LOCALAPPDATA") {
        return Ok(PathBuf::from(path).join("modn/state.sqlite"));
    }
    if let Some(path) = std::env::var_os("XDG_STATE_HOME") {
        return Ok(PathBuf::from(path).join("modn/state.sqlite"));
    }
    let home = std::env::var_os("HOME").context("no HOME or app state directory available")?;
    Ok(PathBuf::from(home).join(".local/state/modn/state.sqlite"))
}

pub(super) fn validate_state_path(
    state_path: &Path,
    selected_game: Option<&str>,
    selected_path: Option<&Path>,
    registered_roots: &[PathBuf],
) -> Result<()> {
    // A missing component followed by '..' can escape ancestor-by-ancestor
    // confinement checks before create_dir_all materializes the path.
    if state_path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        bail!("state directory must not contain parent-directory components");
    }
    let selected_root = selected_path.map(Path::canonicalize).transpose()?;
    if let Ok(metadata) = std::fs::symlink_metadata(state_path) {
        if metadata.file_type().is_symlink() {
            bail!("state database must not be a filesystem link");
        }
    }
    let parent = state_path.parent().context("invalid state path")?;
    let mut directory = if parent.is_absolute() {
        parent.to_path_buf()
    } else {
        std::env::current_dir()?.join(parent)
    };
    loop {
        if let Ok(existing) = directory.canonicalize() {
            if selected_root
                .as_ref()
                .is_some_and(|root| existing.starts_with(root))
                || registered_roots
                    .iter()
                    .any(|root| existing.starts_with(root))
            {
                bail!("state directory must be outside a game installation");
            }
            if let Some(game) = selected_game {
                for ancestor in existing.ancestors() {
                    if recognizes_installation_for(game, ancestor)? {
                        bail!("state directory must be outside a game installation");
                    }
                }
            }
            return Ok(());
        }
        directory = directory
            .parent()
            .context("cannot resolve state directory")?
            .to_path_buf();
    }
}
