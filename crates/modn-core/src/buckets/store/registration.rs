//! Registration file reads and the atomic registration write.
use super::BucketStore;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

impl BucketStore {
    /// Atomically replace one bucket registration file.
    pub(in crate::buckets) fn write_registration(
        &self,
        name: &str,
        registration: &Registration,
    ) -> Result<()> {
        self.check_registry_root()?;
        #[cfg(test)]
        if self
            .fail_next_registration_write
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            bail!("injected registration write failure");
        }
        let dir = self.root.join("registrations");
        if let Ok(meta) = fs::symlink_metadata(&dir) {
            if !meta.is_dir() || meta.file_type().is_symlink() {
                bail!("unsafe bucket registry");
            }
        }
        fs::create_dir_all(&dir)?;
        let mut temp = tempfile::NamedTempFile::new_in(&self.root)?;
        serde_json::to_writer(&mut temp, registration)?;
        temp.as_file().sync_all()?;
        temp.persist(dir.join(format!("{name}.json")))?;
        Ok(())
    }

    pub(in crate::buckets) fn registered_git(
        &self,
        name: &str,
    ) -> Result<Option<crate::buckets::GitCatalogRevision>> {
        if !self.registrations()?.contains_key(name) {
            return Ok(None);
        }
        let registration: Registration =
            bounded_json(&self.root.join("registrations").join(format!("{name}.json")))?;
        registration
            .revision(name)?
            .context("bucket is not a Git catalog")
            .map(Some)
    }
}

const MAX_JSON: u64 = 32 * 1024;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::buckets) struct Registration {
    pub(in crate::buckets) path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::buckets) git: Option<crate::buckets::git::StoredGitCatalogRevision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::buckets) zip: Option<crate::buckets::zip_source::StoredZipCatalogRevision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::buckets) module_trust: Option<crate::buckets::trust::ModuleSourceTrust>,
    /// Mirrors listed by the latest verified revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::buckets) mirrors: Option<Vec<crate::buckets::mirrors::CatalogHost>>,
}

impl Registration {
    pub(in crate::buckets) fn revision(
        &self,
        name: &str,
    ) -> Result<Option<crate::buckets::GitCatalogRevision>> {
        self.git
            .clone()
            .map(|stored| stored.revision(name))
            .transpose()
    }

    pub(in crate::buckets) fn catalog_signer(&self) -> Option<&crate::buckets::CatalogSigner> {
        self.zip
            .as_ref()
            .map(|zip| &zip.signer)
            .or_else(|| self.git.as_ref().and_then(|git| git.signer.as_ref()))
    }
}

pub(in crate::buckets) fn bounded_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    if !fs::symlink_metadata(path)?.file_type().is_file() {
        bail!("bucket metadata must be a regular file: {}", path.display());
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_JSON + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_JSON {
        bail!("bucket metadata exceeds size limit");
    }
    serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid bucket JSON {}", path.display()))
}
