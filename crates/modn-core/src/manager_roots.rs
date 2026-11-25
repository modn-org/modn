use anyhow::Result;
use std::path::{Path, PathBuf};

/// Every manager-owned location, resolved once from the data directory and
/// the `--state-dir` manager profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagerRoots {
    data: PathBuf,
    profile: PathBuf,
    archive_cache: PathBuf,
    development_modules: Option<PathBuf>,
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

    /// Resolve the roots from the environment.
    ///
    /// Reads the data directory and `MODN_CACHE` once. An empty `MODN_CACHE`
    /// keeps the profile cache.
    pub fn from_environment(state_dir: Option<&Path>) -> Result<Self> {
        let data = crate::data_dir::data_directory()?;
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

    /// The shared official `main` bucket copy, which every profile uses.
    #[must_use]
    pub fn official_main(&self) -> PathBuf {
        self.data.join("buckets").join("main")
    }

    /// The development module override, when one is selected.
    #[must_use]
    pub fn development_modules(&self) -> Option<&Path> {
        self.development_modules.as_deref()
    }
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
                official_main: data.join("buckets").join("main"),
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
        assert_eq!(roots.official_main(), data.join("buckets").join("main"));

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
        let roots = crate::data_dir::with_data_directory(temp.path(), || {
            ManagerRoots::from_environment(Some(&state)).expect("resolve manager roots")
        });
        assert_eq!(roots.data(), temp.path());
        assert_eq!(roots.profile(), state.as_path());
        assert_eq!(roots.state_database(), state.join("state.sqlite"));

        let default = crate::data_dir::with_data_directory(temp.path(), || {
            ManagerRoots::from_environment(None).expect("resolve manager roots")
        });
        assert_eq!(default.data(), temp.path());
        assert_eq!(default.profile(), temp.path());
        assert_eq!(default.state_database(), temp.path().join("state.sqlite"));
    }
}
