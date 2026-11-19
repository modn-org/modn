//! Deferred removal of superseded catalog trees.
//!
//! A sync publishes the new registration and then retires the tree that the
//! previous registration named. `BucketStore::snapshot` and `BucketStore::list`
//! read a registration and then open the path it names without the sync lock,
//! so a tree must outlive the switch that replaced it. Retiring records the
//! tree and its bucket in the bucket root, and writes the retire time into the
//! tree itself. A later sync removes recorded trees after its registration
//! write, while it holds the sync lock, but only after `RETIRED_GRACE` has
//! passed. A missing or malformed record never deletes a tree: a tree without
//! a readable retire time gets a fresh marker. An explicit `BucketStore::remove`
//! ends the bucket, so it deletes the live tree and that bucket's retired trees
//! at once.

use super::lock::{acquire_lock, LOCK_WAIT};
use super::manifest::valid_id;
use super::store::{bounded_json, BucketStore, Registration};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Marker file in the bucket root that lists retired catalog trees.
const RETIRED_CATALOGS: &str = ".retired-catalogs";

/// Marker file inside a retired catalog tree that records its retire time.
const RETIRED_MARKER: &str = ".modn-retired";

/// How long a retired catalog tree stays on disk after it is retired.
///
/// A reader may load a registration just before a sync replaces it. The grace
/// period keeps the superseded tree available to that reader even when two
/// syncs run in quick succession.
pub(super) const RETIRED_GRACE: Duration = Duration::from_secs(60 * 60);

/// One tree that a sync retired, with the bucket that owned it.
#[derive(Clone, Debug, Serialize)]
struct RetiredCatalog {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    bucket: Option<String>,
}

/// Retire time stored inside one retired tree.
#[derive(Deserialize, Serialize)]
struct RetiredMarker {
    retired_at: u64,
}

impl BucketStore {
    /// Record one superseded catalog tree for later removal.
    ///
    /// The tree stays on disk, so a reader that already loaded the superseded
    /// registration keeps working. A path outside the bucket root, or one that
    /// no sync created, is ignored. The caller holds the sync lock.
    pub(super) fn retire_catalog_snapshot(&self, bucket: &str, path: &Path) {
        let Some(name) = Self::snapshot_name(&self.root, path) else {
            return;
        };
        let mut retired = self.retired_catalogs();
        if let Some(entry) = retired.iter_mut().find(|entry| entry.name == name) {
            if entry.bucket.is_none() {
                entry.bucket = Some(bucket.to_owned());
                let _ = self.write_retired_catalogs(&retired);
            }
            return;
        }
        let snapshot = self.root.join(&name);
        let _ = self.write_retired_marker(&snapshot, self.retired_now());
        retired.push(RetiredCatalog {
            name,
            bucket: Some(bucket.to_owned()),
        });
        let _ = self.write_retired_catalogs(&retired);
    }

    /// Remove the catalog trees that an earlier sync retired.
    ///
    /// The caller holds the sync lock and has already published its own
    /// registration, so a failed sync never reaches this step. A tree stays
    /// until its recorded retire time is at least `RETIRED_GRACE` old. A tree
    /// that any current registration names is kept, and a tree without a
    /// readable retire time gets a fresh marker instead of an immediate delete.
    /// Removal is best effort: a tree that cannot be removed stays recorded,
    /// because it is waste, not a correctness problem.
    pub(super) fn purge_retired_catalog_snapshots(&self) {
        let mut retired = self.retired_catalogs();
        if retired.is_empty() {
            return;
        }
        let Ok(registrations) = self.registrations() else {
            return;
        };
        let now = self.retired_now();
        retired.retain(|entry| {
            let Some(snapshot) = self.catalog_snapshot_path(&entry.name) else {
                return false;
            };
            if !snapshot.is_dir() {
                return false;
            }
            if registrations
                .values()
                .any(|registration| registration.path.starts_with(&snapshot))
            {
                return true;
            }
            let Some(retired_at) = self.retired_at(&snapshot) else {
                let _ = self.write_retired_marker(&snapshot, now);
                return true;
            };
            if now.saturating_sub(retired_at) < RETIRED_GRACE.as_secs() {
                return true;
            }
            fs::remove_dir_all(&snapshot).is_err()
        });
        if retired.is_empty() {
            let _ = fs::remove_file(self.root.join(RETIRED_CATALOGS));
        } else {
            let _ = self.write_retired_catalogs(&retired);
        }
    }

    /// Forget a registered bucket without touching its source or game files.
    ///
    /// A catalog tree that this store created for the bucket is removed after
    /// the registration file is gone, together with the trees this bucket
    /// retired. A local folder source and a tree that another registration
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
        self.remove_bucket_catalog_trees(name, &registration);
        Ok(())
    }

    /// Remove the live and retired catalog trees of one removed bucket.
    ///
    /// The registration file is already gone. Only a registration that names
    /// a catalog (Git or ZIP) owns its tree; a local folder bucket never does,
    /// whatever its folder is called. A tree that another registration names
    /// stays for that registration.
    fn remove_bucket_catalog_trees(&self, name: &str, registration: &Registration) {
        let Ok(registrations) = self.registrations() else {
            return;
        };
        let referenced = |snapshot: &Path| {
            registrations
                .values()
                .any(|registration| registration.path.starts_with(snapshot))
        };
        if registration.is_catalog() {
            if let Some(snapshot) = Self::snapshot_path(&self.root, &registration.path) {
                if !referenced(&snapshot) {
                    let _ = fs::remove_dir_all(&snapshot);
                }
            }
        }
        let mut retired = self.retired_catalogs();
        let before = retired.len();
        retired.retain(|entry| {
            if entry.bucket.as_deref() != Some(name) {
                return true;
            }
            let Some(snapshot) = self.catalog_snapshot_path(&entry.name) else {
                return false;
            };
            if referenced(&snapshot) {
                return true;
            }
            let _ = fs::remove_dir_all(&snapshot);
            snapshot.exists()
        });
        if retired.is_empty() {
            let _ = fs::remove_file(self.root.join(RETIRED_CATALOGS));
        } else if retired.len() != before {
            let _ = self.write_retired_catalogs(&retired);
        }
    }

    /// The current time in seconds, with the test clock when one is installed.
    fn retired_now(&self) -> u64 {
        #[cfg(test)]
        if let Some(clock) = &self.clock {
            return clock();
        }
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since_epoch| since_epoch.as_secs())
    }

    /// Recorded retired trees. A missing or malformed marker records none.
    fn retired_catalogs(&self) -> Vec<RetiredCatalog> {
        let path = self.root.join(RETIRED_CATALOGS);
        if !path.is_file() {
            return Vec::new();
        }
        let Ok(values) = bounded_json::<Vec<serde_json::Value>>(&path) else {
            return Vec::new();
        };
        values
            .into_iter()
            .filter_map(parse_retired_catalog)
            .collect()
    }

    fn write_retired_catalogs(&self, retired: &[RetiredCatalog]) -> Result<()> {
        let mut temp = tempfile::NamedTempFile::new_in(&self.root)?;
        serde_json::to_writer(&mut temp, retired)?;
        temp.as_file().sync_all()?;
        temp.persist(self.root.join(RETIRED_CATALOGS))?;
        Ok(())
    }

    /// The retire time written inside one tree, or `None` when it is unreadable.
    fn retired_at(&self, snapshot: &Path) -> Option<u64> {
        let marker: RetiredMarker = bounded_json(&snapshot.join(RETIRED_MARKER)).ok()?;
        Some(marker.retired_at)
    }

    /// Write the retire time inside one tree before its removal can be considered.
    fn write_retired_marker(&self, snapshot: &Path, retired_at: u64) -> Result<()> {
        let mut temp = tempfile::NamedTempFile::new_in(snapshot)?;
        serde_json::to_writer(&mut temp, &RetiredMarker { retired_at })?;
        temp.as_file().sync_all()?;
        temp.persist(snapshot.join(RETIRED_MARKER))?;
        Ok(())
    }

    /// A recorded name must be one plain path component under the bucket root.
    fn catalog_snapshot_path(&self, name: &str) -> Option<PathBuf> {
        let mut components = Path::new(name).components();
        match (components.next(), components.next()) {
            (Some(Component::Normal(_)), None) => Some(self.root.join(name)),
            _ => None,
        }
    }

    /// The tree root that contains `path`, when it sits directly in this store.
    fn snapshot_name(root: &Path, path: &Path) -> Option<String> {
        let snapshot = path
            .ancestors()
            .find(|candidate| candidate.parent() == Some(root))?;
        Some(snapshot.file_name()?.to_string_lossy().into_owned())
    }

    fn snapshot_path(root: &Path, path: &Path) -> Option<PathBuf> {
        Self::snapshot_name(root, path).map(|name| root.join(name))
    }
}

/// Parse one retire record, including a name-only record from an older release.
fn parse_retired_catalog(value: serde_json::Value) -> Option<RetiredCatalog> {
    if let Some(name) = value.as_str() {
        return Some(RetiredCatalog {
            name: name.to_owned(),
            bucket: None,
        });
    }
    let object = value.as_object()?;
    let name = object.get("name")?.as_str()?.to_owned();
    let bucket = object
        .get("bucket")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    Some(RetiredCatalog { name, bucket })
}
