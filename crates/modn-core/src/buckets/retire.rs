//! Deferred removal of superseded catalog trees.
//!
//! A sync publishes the new registration and then retires the tree that the
//! previous registration named. `BucketStore::snapshot` and `BucketStore::list`
//! read a registration and then open the path it names without the sync lock,
//! so a tree must outlive the switch that replaced it. Retiring records the
//! tree; the next sync removes recorded trees while it holds the sync lock, and
//! it never removes a tree that a registration names.

use super::lock::{acquire_lock, LOCK_WAIT};
use super::manifest::valid_id;
use super::store::bounded_json;
use super::BucketStore;
use anyhow::{bail, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Marker file in the bucket root that lists retired catalog trees.
const RETIRED_CATALOGS: &str = ".retired-catalogs";

impl BucketStore {
    /// Record one superseded catalog tree for removal by the next sync.
    ///
    /// The tree stays on disk, so a reader that already loaded the superseded
    /// registration keeps working. A path outside the bucket root, or one that
    /// no sync created, is ignored. The caller holds the sync lock.
    pub(super) fn retire_catalog_snapshot(&self, path: &Path) {
        let Some(name) = Self::owned_catalog_snapshot_name(&self.root, path) else {
            return;
        };
        let mut retired = self.retired_catalog_snapshots();
        if retired.contains(&name) {
            return;
        }
        retired.push(name);
        let _ = self.write_retired_catalog_snapshots(&retired);
    }

    /// Remove the catalog trees that an earlier sync retired.
    ///
    /// The caller holds the sync lock. A tree that any current registration
    /// names is kept for a later sync. Removal is best effort: a tree that
    /// cannot be removed stays recorded, because it is waste, not a
    /// correctness problem.
    pub(super) fn purge_retired_catalog_snapshots(&self) {
        let retired = self.retired_catalog_snapshots();
        if retired.is_empty() {
            return;
        }
        let Ok(registrations) = self.registrations() else {
            return;
        };
        let mut remaining = Vec::new();
        for name in retired {
            let Some(snapshot) = self.catalog_snapshot_path(&name) else {
                continue;
            };
            let referenced = registrations
                .values()
                .any(|registration| registration.path.starts_with(&snapshot));
            if referenced || fs::remove_dir_all(&snapshot).is_err() {
                remaining.push(name);
            }
        }
        if remaining.is_empty() {
            let _ = fs::remove_file(self.root.join(RETIRED_CATALOGS));
        } else {
            let _ = self.write_retired_catalog_snapshots(&remaining);
        }
    }

    /// Remove the catalog tree of a bucket whose registration is already gone.
    ///
    /// The caller holds the sync lock. An explicit removal ends the bucket, so
    /// its own tree can go at once. A local folder source and a tree that
    /// another registration still names are kept.
    pub(super) fn remove_retired_catalog_snapshot(&self, path: &Path) {
        let Some(name) = Self::owned_catalog_snapshot_name(&self.root, path) else {
            return;
        };
        let snapshot = self.root.join(name);
        let Ok(registrations) = self.registrations() else {
            return;
        };
        if registrations
            .values()
            .any(|registration| registration.path.starts_with(&snapshot))
        {
            return;
        }
        let _ = fs::remove_dir_all(&snapshot);
    }

    /// Forget a registered bucket without touching its source or game files.
    ///
    /// A catalog tree that this store created is removed after the registration
    /// file is gone. A local folder source and a tree that another registration
    /// still names are never removed.
    pub fn remove(&self, name: &str) -> Result<()> {
        if !valid_id(name) || name == "main" {
            bail!("cannot remove the reserved main bucket");
        }
        if !self.registrations()?.contains_key(name) {
            bail!("bucket is not registered: {name}");
        }
        fs::create_dir_all(&self.root)?;
        let _lock = acquire_lock(
            &self.root.join(".git-sync.lock"),
            "unsafe bucket lock",
            "bucket sync already running",
            LOCK_WAIT,
        )?;
        let registration = self.registration(name)?;
        #[cfg(test)]
        if self
            .fail_next_registration_remove
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            bail!("injected registration removal failure");
        }
        let file = self.root.join("registrations").join(format!("{name}.json"));
        fs::remove_file(file)?;
        self.remove_retired_catalog_snapshot(&registration.path);
        Ok(())
    }

    /// Recorded retired tree names. A missing or malformed marker deletes nothing.
    fn retired_catalog_snapshots(&self) -> Vec<String> {
        let path = self.root.join(RETIRED_CATALOGS);
        if !path.is_file() {
            return Vec::new();
        }
        bounded_json(&path).unwrap_or_default()
    }

    fn write_retired_catalog_snapshots(&self, names: &[String]) -> Result<()> {
        let mut temp = tempfile::NamedTempFile::new_in(&self.root)?;
        serde_json::to_writer(&mut temp, names)?;
        temp.as_file().sync_all()?;
        temp.persist(self.root.join(RETIRED_CATALOGS))?;
        Ok(())
    }

    /// A recorded name must be one path component of a tree that a sync created.
    fn catalog_snapshot_path(&self, name: &str) -> Option<PathBuf> {
        let one_component = Path::new(name).components().count() == 1;
        if !one_component || !Self::catalog_snapshot_name(name) {
            return None;
        }
        Some(self.root.join(name))
    }

    /// The tree root that contains `path`, when a sync created it in this store.
    fn owned_catalog_snapshot_name(root: &Path, path: &Path) -> Option<String> {
        let snapshot = path
            .ancestors()
            .find(|candidate| candidate.parent() == Some(root))?;
        let name = snapshot.file_name()?.to_string_lossy().into_owned();
        Self::catalog_snapshot_name(&name).then_some(name)
    }

    fn catalog_snapshot_name(name: &str) -> bool {
        name.starts_with("zip-catalog-") || name.starts_with("git-catalog-")
    }
}
