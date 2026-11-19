//! Registration file reads and the atomic registration write.
use super::BucketStore;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
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
        let bytes = serde_json::to_vec(registration)?;
        if bytes.len() as u64 > MAX_JSON {
            bail!("bucket registration exceeds size limit");
        }
        self.write_registration_bytes(name, &bytes)
    }

    /// Atomically write exact registration bytes, bypassing the test seam.
    fn write_registration_bytes(&self, name: &str, bytes: &[u8]) -> Result<()> {
        let dir = self.root.join("registrations");
        if let Ok(meta) = fs::symlink_metadata(&dir) {
            if !meta.is_dir() || meta.file_type().is_symlink() {
                bail!("unsafe bucket registry");
            }
        }
        fs::create_dir_all(&dir)?;
        let mut temp = tempfile::NamedTempFile::new_in(&self.root)?;
        temp.as_file_mut().write_all(bytes)?;
        temp.as_file().sync_all()?;
        temp.persist(dir.join(format!("{name}.json")))?;
        Ok(())
    }

    /// The stored registration bytes, or `None` when the bucket is not registered.
    pub(in crate::buckets) fn stored_registration_bytes(
        &self,
        name: &str,
    ) -> Result<Option<Vec<u8>>> {
        match fs::read(self.root.join("registrations").join(format!("{name}.json"))) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Put a registration file back exactly as it was, or remove it when it did not exist.
    pub(in crate::buckets) fn restore_registration(
        &self,
        name: &str,
        bytes: Option<&[u8]>,
    ) -> Result<()> {
        #[cfg(test)]
        if self
            .fail_next_registration_restore
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            bail!("injected registration restore failure");
        }
        match bytes {
            Some(bytes) => self.write_registration_bytes(name, bytes),
            None => {
                let path = self.root.join("registrations").join(format!("{name}.json"));
                match fs::remove_file(path) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(error.into()),
                }
            }
        }
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
    /// True when this registration names a catalog tree that a sync created.
    pub(in crate::buckets) fn is_catalog(&self) -> bool {
        self.git.is_some() || self.zip.is_some()
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn registration(path: &str) -> Registration {
        Registration {
            path: PathBuf::from(path),
            git: None,
            zip: None,
            module_trust: None,
            mirrors: None,
        }
    }

    #[test]
    fn an_oversized_registration_leaves_the_stored_file_unchanged() {
        let directory = tempfile::tempdir().expect("fixture");
        let store = BucketStore::new(directory.path());
        store
            .write_registration("community", &registration("first"))
            .expect("initial registration");
        let path = directory
            .path()
            .join("buckets/registrations/community.json");
        let before = std::fs::read(&path).expect("stored registration");

        let error = store
            .write_registration("community", &registration(&"x".repeat(40_000)))
            .expect_err("oversized registration")
            .to_string();

        assert!(error.contains("exceeds size limit"), "{error}");
        assert_eq!(std::fs::read(&path).expect("stored registration"), before);
    }
}
