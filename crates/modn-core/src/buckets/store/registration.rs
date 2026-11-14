//! Registration file reads and the atomic registration write.
use super::{bounded_json, BucketStore, Registration};
use anyhow::{bail, Context, Result};
use std::fs;

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
    ) -> Result<Option<super::super::GitCatalogRevision>> {
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
