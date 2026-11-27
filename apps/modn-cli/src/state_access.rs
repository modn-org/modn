use crate::selection::selected_installation;
use anyhow::{bail, Context, Result};
use modn_core::buckets::BucketStore;
use modn_core::discovery::recognizes_installation_for;
use modn_core::game_modules::ModuleSelection;
use modn_core::manager_roots::ManagerRoots;
use modn_core::state::Store;
use std::path::{Component, Path, PathBuf};

pub(super) struct ManagerState {
    roots: ManagerRoots,
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
        let roots = state.roots.clone();
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
            None => {
                selected_installation(&roots, store.as_ref(), target.root, target.game.as_deref())?
            }
        };
        Ok(SelectedState {
            store,
            game_id,
            root,
        })
    }
}

impl ManagerState {
    pub(super) fn load(roots: &ManagerRoots) -> Result<Self> {
        let mut state = Self::load_deferred(roots)?;
        state.registered_roots = Some(registered_roots(state.existing())?);
        Ok(state)
    }

    pub(super) fn load_deferred(roots: &ManagerRoots) -> Result<Self> {
        let path = roots.state_database();
        let existing = path
            .is_file()
            .then(|| Store::open_readonly(&path))
            .transpose()?;
        Ok(Self {
            roots: roots.clone(),
            path,
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
        roots: &ManagerRoots,
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
            let (game_id, root) = selected_installation(roots, self.existing(), None, requested)?;
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
            &self.roots,
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
            &self.roots,
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

pub(super) fn validate_write_targets(roots: &ManagerRoots, extra_game: Option<&str>) -> Result<()> {
    let state = ManagerState::load_deferred(roots)?;
    let registered = state
        .existing()
        .map(Store::list)
        .transpose()?
        .unwrap_or_default()
        .into_iter()
        .map(|entry| entry.path)
        .collect::<Vec<_>>();
    let mut games = BucketStore::new(roots).official_games()?;
    if let Some(game) = extra_game {
        games.push(game.to_owned());
    }
    games.sort();
    games.dedup();
    let targets = roots.write_targets();
    for target in &targets {
        validate_state_path(roots, target, None, None, &registered)?;
    }
    for game in games {
        for target in &targets {
            validate_state_path(roots, target, Some(&game), None, &registered)?;
        }
    }
    Ok(())
}

pub(super) fn validate_state_path(
    roots: &ManagerRoots,
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
        roots,
        "state directory",
        parent,
        selected_game,
        selected_path,
        registered_roots,
    )
}

pub(super) fn validate_outside_games(
    roots: &ManagerRoots,
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
    let selection = selected_game
        .map(|game| ModuleSelection::resolve(roots, game))
        .transpose()?;
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
            if let Some(selection) = &selection {
                for ancestor in existing.ancestors() {
                    if recognizes_installation_for(selection, ancestor)? {
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
