use crate::selection::selected_installation;
use anyhow::{bail, Context, Result};
use modn_core::discovery::recognizes_installation_for;
use modn_core::state::Store;
use std::path::{Component, Path, PathBuf};

pub(super) struct ManagerState {
    path: PathBuf,
    existing: Option<Store>,
    registered_roots: Option<Vec<PathBuf>>,
}

#[derive(Default)]
pub(super) struct ProtectedTarget {
    pub(super) game: Option<String>,
    pub(super) root: Option<PathBuf>,
}

pub(super) enum InstallationAccess {
    Preview,
    Write,
    ArchivePreview,
    ArchiveInstall { approved: bool },
}

pub(super) struct InstallationState {
    state: ManagerState,
    target: ProtectedTarget,
    selected: Option<(String, PathBuf)>,
}

pub(super) struct SelectedState {
    pub(super) store: Option<Store>,
    pub(super) game_id: String,
    pub(super) root: PathBuf,
}

impl InstallationState {
    pub(super) fn open(self, access: InstallationAccess) -> Result<SelectedState> {
        let Self {
            state,
            target,
            selected,
        } = self;
        let store = match access {
            InstallationAccess::ArchivePreview
                if target.root.is_some() && state.existing().is_none() =>
            {
                state.validate(&target)?;
                None
            }
            InstallationAccess::Preview | InstallationAccess::ArchivePreview => {
                Some(state.open_preview(&target)?)
            }
            InstallationAccess::Write | InstallationAccess::ArchiveInstall { .. } => {
                Some(state.open_writable(&target)?)
            }
        };
        if let InstallationAccess::ArchiveInstall { approved: false } = access {
            bail!(
                "installation writes game files; choose --dry-run or explicitly approve with --yes"
            );
        }
        let (game_id, root) = match selected {
            Some(selected) => selected,
            None => selected_installation(store.as_ref(), target.root, target.game.as_deref())?,
        };
        Ok(SelectedState {
            store,
            game_id,
            root,
        })
    }
}

impl ManagerState {
    pub(super) fn load(path: &Path) -> Result<Self> {
        let mut state = Self::load_deferred(path)?;
        state.registered_roots = Some(registered_roots(state.existing())?);
        Ok(state)
    }

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

    pub(super) fn for_installation(
        self,
        path: Option<PathBuf>,
        requested: Option<&str>,
    ) -> Result<InstallationState> {
        let (target, selected) = if let Some(path) = path {
            (
                ProtectedTarget {
                    game: requested.map(str::to_owned),
                    root: Some(path),
                },
                None,
            )
        } else {
            let (game_id, root) = selected_installation(self.existing(), None, requested)?;
            (
                ProtectedTarget {
                    game: Some(game_id.clone()),
                    root: Some(root.clone()),
                },
                Some((game_id, root)),
            )
        };
        Ok(InstallationState {
            state: self,
            target,
            selected,
        })
    }

    pub(super) fn open_writable(self, target: &ProtectedTarget) -> Result<Store> {
        self.validate(target)?;
        Store::open(&self.path)
    }

    pub(super) fn open_preview(self, target: &ProtectedTarget) -> Result<Store> {
        self.validate(target)?;
        self.existing
            .context("no registered game database for preview")
    }

    pub(super) fn validate_download(&self, cache: &Path, target: &ProtectedTarget) -> Result<()> {
        self.validate(target)?;
        validate_outside_games(
            "cache directory",
            cache,
            target.game.as_deref(),
            target.root.as_deref(),
            &self.roots()?,
        )
    }

    fn roots(&self) -> Result<Vec<PathBuf>> {
        match &self.registered_roots {
            Some(roots) => Ok(roots.clone()),
            None => registered_roots(self.existing()),
        }
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
    Ok(modn_core::data_dir::data_directory()?.join("state.sqlite"))
}

pub(super) fn validate_state_path(
    state_path: &Path,
    selected_game: Option<&str>,
    selected_path: Option<&Path>,
    registered_roots: &[PathBuf],
) -> Result<()> {
    if let Ok(metadata) = std::fs::symlink_metadata(state_path) {
        if metadata.file_type().is_symlink() {
            bail!("state database must not be a filesystem link");
        }
    }
    let parent = state_path.parent().context("invalid state path")?;
    validate_outside_games(
        "state directory",
        parent,
        selected_game,
        selected_path,
        registered_roots,
    )
}

pub(super) fn validate_outside_games(
    label: &str,
    parent: &Path,
    selected_game: Option<&str>,
    selected_path: Option<&Path>,
    registered_roots: &[PathBuf],
) -> Result<()> {
    if parent
        .components()
        .any(|component| component == Component::ParentDir)
    {
        bail!("{label} must not contain parent-directory components");
    }
    let selected_root = selected_path.map(Path::canonicalize).transpose()?;
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
                bail!("{label} must be outside a game installation");
            }
            if let Some(game) = selected_game {
                for ancestor in existing.ancestors() {
                    if recognizes_installation_for(game, ancestor)? {
                        bail!("{label} must be outside a game installation");
                    }
                }
            }
            return Ok(());
        }
        directory = directory
            .parent()
            .with_context(|| format!("cannot resolve {label}"))?
            .to_path_buf();
    }
}
