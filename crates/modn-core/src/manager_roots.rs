use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Every manager-owned location, resolved once from the data directory and
/// the `--state-dir` manager profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagerRoots {
    data: PathBuf,
    profile: PathBuf,
    archive_cache: PathBuf,
    development_modules: Option<PathBuf>,
    development_module_base: Option<PathBuf>,
}

impl ManagerRoots {
    /// Resolve the roots without reading the environment.
    ///
    /// Without a state directory the profile is the data directory.
    #[must_use]
    pub fn new(data: &Path, state_dir: Option<&Path>) -> Self {
        let profile = state_dir.map_or_else(|| data.to_path_buf(), Path::to_path_buf);
        let archive_cache = profile.join("cache");
        Self {
            data: data.to_path_buf(),
            profile,
            archive_cache,
            development_modules: None,
            development_module_base: None,
        }
    }

    /// Use an explicit archive cache root instead of the profile cache.
    #[must_use]
    pub fn with_archive_cache(mut self, root: PathBuf) -> Self {
        self.archive_cache = root;
        self
    }

    /// Select or clear the development module override.
    #[must_use]
    pub fn with_development_modules(mut self, root: Option<PathBuf>) -> Self {
        self.development_modules = root;
        self
    }

    /// Select or clear the development module base directory.
    ///
    /// The selection joins this base with the game ID, so one base can hold
    /// several game modules. The caller passes it explicitly; this value never
    /// reads the process environment.
    #[must_use]
    pub fn with_development_module_base(mut self, root: Option<PathBuf>) -> Self {
        self.development_module_base = root;
        self
    }

    /// Resolve the roots from the environment.
    ///
    /// Reads the data directory and `MODN_CACHE` once. An empty `MODN_CACHE`
    /// keeps the profile cache.
    pub fn from_environment(state_dir: Option<&Path>) -> Result<Self> {
        let data = default_data_directory()?;
        let mut roots = Self::new(&data, state_dir);
        if let Some(directory) = std::env::var_os("MODN_CACHE").filter(|value| !value.is_empty()) {
            roots.archive_cache = PathBuf::from(directory);
        }
        Ok(roots)
    }

    /// The shared data directory.
    #[must_use]
    pub fn data(&self) -> &Path {
        &self.data
    }

    /// The manager profile selected by `--state-dir`, or the data directory.
    #[must_use]
    pub fn profile(&self) -> &Path {
        &self.profile
    }

    /// The manager state database.
    #[must_use]
    pub fn state_database(&self) -> PathBuf {
        self.profile.join("state.sqlite")
    }

    /// The archive cache root.
    #[must_use]
    pub fn archive_cache_root(&self) -> &Path {
        &self.archive_cache
    }

    /// The archive cache rooted at [`Self::archive_cache_root`].
    #[must_use]
    pub fn archive_cache(&self) -> crate::acquisition::ArchiveCache {
        crate::acquisition::ArchiveCache::new(self.archive_cache.clone())
    }

    /// The bucket store root, which holds the bucket registrations.
    #[must_use]
    pub fn bucket_registrations(&self) -> PathBuf {
        self.profile.join("buckets")
    }

    /// The game-module registry root.
    #[must_use]
    pub fn module_registry(&self) -> PathBuf {
        self.profile.join("game-modules")
    }

    /// The leaf paths that Modn writes under this profile.
    #[must_use]
    pub fn write_targets(&self) -> Vec<PathBuf> {
        vec![
            self.state_database(),
            self.bucket_registrations()
                .join("registrations")
                .join(".bucket-entry.json"),
            self.module_registry().join("registry.lock"),
        ]
    }

    /// The bucket root inside the manager profile that holds the official
    /// `main` copy.
    ///
    /// Without `--state-dir` this is the shared data bucket root.
    #[must_use]
    pub fn official_bucket_root(&self) -> PathBuf {
        self.profile.join("buckets")
    }

    /// The official `main` bucket copy inside the manager profile.
    ///
    /// A selected `--state-dir` keeps the copy under that directory, so the
    /// profile is self-contained.
    #[must_use]
    pub fn official_main(&self) -> PathBuf {
        self.official_bucket_root().join("main")
    }

    /// The development module override, when one is selected.
    #[must_use]
    pub fn development_modules(&self) -> Option<&Path> {
        self.development_modules.as_deref()
    }

    /// The development module base directory, when one is selected.
    #[must_use]
    pub fn development_module_base(&self) -> Option<&Path> {
        self.development_module_base.as_deref()
    }
}

/// Resolve the shared data directory from the state-directory environment overrides.
fn default_data_directory() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("LOCALAPPDATA") {
        return Ok(PathBuf::from(path).join("modn"));
    }
    if let Some(path) = std::env::var_os("XDG_STATE_HOME") {
        return Ok(PathBuf::from(path).join("modn"));
    }
    let home = std::env::var_os("HOME").context("no HOME or app state directory available")?;
    Ok(PathBuf::from(home).join(".local/state/modn"))
}

#[cfg(test)]
mod tests {
    use super::ManagerRoots;
    use std::path::{Path, PathBuf};

    #[test]
    fn table_resolves_default_and_state_dir_profiles() {
        struct Case {
            state_dir: Option<PathBuf>,
            profile: PathBuf,
            official_main: PathBuf,
        }

        let data = PathBuf::from("/manager-roots/data");
        let cases = [
            Case {
                state_dir: None,
                profile: data.clone(),
                official_main: data.join("buckets").join("main"),
            },
            Case {
                state_dir: Some(PathBuf::from("/manager-roots/profile")),
                profile: PathBuf::from("/manager-roots/profile"),
                official_main: PathBuf::from("/manager-roots/profile")
                    .join("buckets")
                    .join("main"),
            },
        ];

        for case in cases {
            let roots = ManagerRoots::new(&data, case.state_dir.as_deref());
            assert_eq!(roots.profile(), case.profile.as_path());
            assert_eq!(roots.state_database(), case.profile.join("state.sqlite"));
            assert_eq!(roots.archive_cache_root(), case.profile.join("cache"));
            assert_eq!(roots.bucket_registrations(), case.profile.join("buckets"));
            assert_eq!(roots.module_registry(), case.profile.join("game-modules"));
            assert_eq!(roots.official_main(), case.official_main);
            assert_eq!(
                roots.official_bucket_root(),
                case.official_main
                    .parent()
                    .expect("official main has a parent")
            );
        }
    }

    #[test]
    fn write_targets_enumerate_the_profile_leaves() {
        let data = Path::new("/manager-roots/data");
        let cases = [
            (None, PathBuf::from("/manager-roots/data")),
            (
                Some(Path::new("/manager-roots/profile")),
                PathBuf::from("/manager-roots/profile"),
            ),
        ];

        for (state_dir, profile) in cases {
            let roots = ManagerRoots::new(data, state_dir);
            assert_eq!(
                roots.write_targets(),
                vec![
                    profile.join("state.sqlite"),
                    profile
                        .join("buckets")
                        .join("registrations")
                        .join(".bucket-entry.json"),
                    profile.join("game-modules").join("registry.lock"),
                ]
            );
        }
    }

    #[test]
    fn builders_replace_the_archive_cache_and_development_modules() {
        let data = Path::new("/manager-roots/data");
        let roots = ManagerRoots::new(data, Some(Path::new("/manager-roots/profile")))
            .with_archive_cache(PathBuf::from("/manager-roots/archive-cache"))
            .with_development_modules(Some(PathBuf::from("/manager-roots/dev-modules")));

        assert_eq!(
            roots.archive_cache_root(),
            Path::new("/manager-roots/archive-cache")
        );
        assert_eq!(
            roots.development_modules(),
            Some(Path::new("/manager-roots/dev-modules"))
        );
        assert_eq!(roots.profile(), Path::new("/manager-roots/profile"));
        assert_eq!(
            roots.official_main(),
            Path::new("/manager-roots/profile")
                .join("buckets")
                .join("main")
        );

        let cleared = roots.with_development_modules(None);
        assert_eq!(cleared.development_modules(), None);
        assert_eq!(
            cleared.archive_cache_root(),
            Path::new("/manager-roots/archive-cache")
        );
    }

    #[test]
    fn from_environment_reads_data_and_profile_only() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let state = temp.path().join("profile");
        let previous = std::env::var_os("LOCALAPPDATA");
        std::env::set_var("LOCALAPPDATA", temp.path());
        let roots = ManagerRoots::from_environment(Some(&state));
        let default = ManagerRoots::from_environment(None);
        match previous {
            Some(value) => std::env::set_var("LOCALAPPDATA", value),
            None => std::env::remove_var("LOCALAPPDATA"),
        }
        let roots = roots.expect("resolve manager roots");
        let default = default.expect("resolve manager roots");
        assert_eq!(roots.data(), temp.path().join("modn").as_path());
        assert_eq!(roots.profile(), state.as_path());
        assert_eq!(roots.state_database(), state.join("state.sqlite"));
        assert_eq!(
            roots.official_main(),
            state.join("buckets").join("main"),
            "the selected profile holds its own official main copy"
        );

        assert_eq!(default.data(), temp.path().join("modn").as_path());
        assert_eq!(default.profile(), temp.path().join("modn").as_path());
        assert_eq!(
            default.state_database(),
            temp.path().join("modn").join("state.sqlite")
        );
        assert_eq!(
            default.official_main(),
            temp.path().join("modn").join("buckets").join("main")
        );
    }
}
