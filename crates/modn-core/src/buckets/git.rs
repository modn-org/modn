use super::manifest::valid_id;
use super::BucketStore;
use anyhow::{bail, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;

/// An explicitly selected community catalog remote and branch, tag or commit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct GitCatalogSource {
    /// HTTPS remote or absolute local path. Local paths support offline fixtures.
    pub remote: String,
    /// Exact Git revision expression. Option-like and reflog expressions are refused.
    pub reference: String,
}

/// Source and commit published together with a validated catalog pointer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct GitCatalogRevision {
    /// Explicit source retained for subsequent updates.
    pub source: GitCatalogSource,
    /// Full resolved Git commit identifier.
    pub commit: String,
}

fn git(directory: &Path, arguments: &[&str]) -> Result<String> {
    let result = Command::new("git")
        .arg("-c")
        .arg("protocol.ext.allow=never")
        .arg("-c")
        .arg("core.hooksPath=")
        .arg("-c")
        .arg("credential.interactive=false")
        .args(arguments)
        .current_dir(directory)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .context("cannot run Git")?;
    if !result.status.success() {
        bail!(
            "Git catalog operation failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    Ok(String::from_utf8(result.stdout)?.trim().to_owned())
}

fn validate_tree(root: &Path) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() || (!kind.is_dir() && !kind.is_file()) {
            bail!("Git catalog contains a non-regular entry");
        }
        if entry.file_name() == "module.json" || entry.file_name() == "module.lua" {
            bail!("community Git catalogs cannot contain game modules");
        }
        if kind.is_dir() {
            validate_tree(&entry.path())?;
        }
    }
    Ok(())
}

impl GitCatalogSource {
    /// Construct a source. Sync validates it before running Git.
    #[must_use]
    pub fn new(remote: String, reference: String) -> Self {
        Self { remote, reference }
    }

    fn validate(&self) -> Result<()> {
        let local = PathBuf::from(&self.remote);
        let https = self.remote.strip_prefix("https://").is_some_and(|rest| {
            let host = rest.split('/').next().unwrap_or_default();
            !host.is_empty() && !host.contains(['@', '?', '#', '\\', ':'])
        });
        if (!https && !local.is_absolute())
            || self.remote.chars().any(char::is_control)
            || self.remote.contains('\\') && https
            || self.reference.is_empty()
            || self.reference.len() > 200
            || self.reference.starts_with('-')
            || self.reference.contains("..")
            || !self
                .reference
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b))
        {
            bail!("invalid Git catalog remote or revision");
        }
        Ok(())
    }
}

impl BucketStore {
    /// Read the published Git revision without fetching. Local registrations return an error.
    pub fn git_revision(&self, name: &str) -> Result<Option<GitCatalogRevision>> {
        if !valid_id(name) || name == "main" {
            bail!("invalid or reserved Git catalog name");
        }
        self.registered_git(name)
    }

    /// Explicitly fetch, check out and validate a community catalog before publication.
    /// New catalogs require `source`; updates reuse the registered source when it is absent.
    /// A failed fetch, checkout or validation leaves the old registration and tree intact.
    /// Publication atomically replaces one registration file. Old trees are retained.
    pub fn sync_git(
        &self,
        name: &str,
        source: Option<GitCatalogSource>,
    ) -> Result<GitCatalogRevision> {
        self.check_registry_root()?;
        if !valid_id(name) || name == "main" {
            bail!("invalid or reserved Git catalog name");
        }
        fs::create_dir_all(&self.root)?;
        let lock_path = self.root.join(".git-sync.lock");
        if let Ok(meta) = fs::symlink_metadata(&lock_path) {
            if !meta.is_file() || meta.file_type().is_symlink() {
                bail!("unsafe Git catalog lock");
            }
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(lock_path)?;
        lock.try_lock_exclusive()
            .context("Git catalog sync already running")?;
        let previous = self.git_revision(name)?;
        let source = match (source, previous) {
            (Some(source), None) => source,
            (None, Some(previous)) => previous.source,
            (Some(source), Some(previous)) if source == previous.source => source,
            _ => {
                bail!("provide a source for registration; changing a registered source is refused")
            }
        };
        source.validate()?;
        let stage = tempfile::Builder::new()
            .prefix("git-catalog-")
            .tempdir_in(&self.root)?;
        git(
            stage.path(),
            &[
                "clone",
                "--no-local",
                "--no-checkout",
                "--",
                &source.remote,
                "catalog",
            ],
        )?;
        let catalog = stage.path().join("catalog");
        git(&catalog, &["checkout", "--detach", &source.reference, "--"])?;
        let commit = git(&catalog, &["rev-parse", "--verify", "HEAD"])?;
        fs::remove_dir_all(catalog.join(".git"))?;
        validate_tree(&catalog)?;
        let revision = GitCatalogRevision { source, commit };
        self.publish_git(name, catalog, revision.clone())?;
        let _retained = stage.keep();
        Ok(revision)
    }
}

#[cfg(test)]
mod tests;
