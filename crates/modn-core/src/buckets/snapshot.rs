use super::{dependencies, BucketPackage};
use crate::transactions::{CatalogConflict, CatalogDependency, CatalogReference, CatalogRelease};
use anyhow::{bail, Context, Result};

/// Validated catalog releases retained for one command without further bucket reads.
#[derive(Clone, Debug)]
pub struct BucketSnapshot {
    current: Vec<BucketPackage>,
    other: Vec<BucketPackage>,
}

impl BucketPackage {
    /// Return the bucket-qualified identity without a version or archive digest.
    #[must_use]
    pub fn reference(&self) -> CatalogReference {
        CatalogReference::new(
            self.bucket.clone(),
            self.package.game.clone(),
            self.package.id.clone(),
        )
    }
}

impl BucketSnapshot {
    pub(super) fn new(mut current: Vec<BucketPackage>, mut other: Vec<BucketPackage>) -> Self {
        let key = |p: &BucketPackage| {
            (
                p.bucket.clone(),
                p.package.game.clone(),
                p.package.id.clone(),
                p.package.version.clone(),
            )
        };
        current.sort_by_key(key);
        other.sort_by_key(key);
        Self { current, other }
    }

    /// Current releases only, in bucket, game and mod order.
    #[must_use]
    pub fn packages(&self) -> &[BucketPackage] {
        &self.current
    }

    /// Match mod, game/mod or bucket/game/mod references against current releases.
    /// The caller must resolve ambiguity before constructing installation evidence.
    pub fn candidates(&self, query: &str) -> Result<Vec<BucketPackage>> {
        let parts: Vec<&str> = query.split('/').collect();
        if !(1..=3).contains(&parts.len()) || parts.iter().any(|part| part.is_empty()) {
            bail!("invalid mod reference; expected mod, game/mod or bucket/game/mod");
        }
        Ok(self
            .current
            .iter()
            .filter(|package| match parts.as_slice() {
                [id] => package.package.id == *id,
                [game, id] => package.package.game == *game && package.package.id == *id,
                [bucket, game, id] => {
                    package.bucket == *bucket
                        && package.package.game == *game
                        && package.package.id == *id
                }
                _ => false,
            })
            .cloned()
            .collect())
    }

    /// Return current and historical releases for one qualified mod, current first.
    pub fn releases(&self, bucket: &str, game: &str, id: &str) -> Result<Vec<BucketPackage>> {
        let releases: Vec<BucketPackage> = self
            .current
            .iter()
            .chain(&self.other)
            .filter(|p| p.bucket == bucket && p.package.game == game && p.package.id == id)
            .cloned()
            .collect();
        if releases.is_empty() {
            bail!("bucket package not found: {bucket}/{game}/{id}");
        }
        Ok(releases)
    }

    /// Resolve dependencies before their dependent using only current snapshot releases.
    pub fn install_order(&self, selected: &BucketPackage) -> Result<Vec<BucketPackage>> {
        dependencies::install_order(selected, &self.current)
    }

    /// Construct pinned dependency and conflict evidence from this snapshot.
    /// Historical selected releases use current dependency releases, like plain installs.
    pub fn release(&self, selected: &BucketPackage) -> Result<CatalogRelease> {
        let published = |id: &str| {
            self.current.iter().find(|candidate| {
                candidate.bucket == selected.bucket
                    && candidate.package.game == selected.package.game
                    && candidate.package.id == id
            })
        };
        let dependencies = selected
            .package
            .dependencies
            .iter()
            .map(|id| {
                let dependency = published(id).with_context(|| {
                    format!(
                        "dependency unavailable in bucket: {}/{}/{}",
                        selected.bucket, selected.package.game, id
                    )
                })?;
                Ok(CatalogDependency::new(
                    dependency.reference(),
                    dependency.package.sha256.clone(),
                )
                .with_requirement(selected.package.dependency_versions.get(id).cloned()))
            })
            .collect::<Result<_>>()?;
        let conflicts = selected
            .package
            .conflicts
            .iter()
            .map(|id| {
                CatalogConflict::new(
                    CatalogReference::new(
                        selected.bucket.clone(),
                        selected.package.game.clone(),
                        id.clone(),
                    ),
                    published(id).map(|candidate| candidate.package.sha256.clone()),
                )
            })
            .collect();
        Ok(CatalogRelease::new(selected.reference(), dependencies)
            .with_conflicts(conflicts)
            .with_version(selected.package.version.clone()))
    }
}
