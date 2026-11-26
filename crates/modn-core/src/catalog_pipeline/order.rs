//! Resolve the install order and its catalog evidence from one snapshot.

use crate::buckets::{BucketPackage, BucketSnapshot};
use crate::transactions::CatalogRelease;
use anyhow::Result;

/// One release of an install order with its snapshot evidence.
#[derive(Debug)]
pub(super) struct OrderedRelease {
    /// Published release, kept for version, sources, executables and format.
    pub(super) package: BucketPackage,
    /// Pinned dependency and conflict evidence from the one snapshot.
    pub(super) release: CatalogRelease,
}

/// Resolve dependencies before their dependent, with the selected release last.
///
/// Reads one snapshot and no other source. A cycle, an unpublished dependency,
/// an unmet version requirement or two conflicting releases is an error, never
/// a partial order.
pub(super) fn resolve(
    catalog: &BucketSnapshot,
    selected: &BucketPackage,
) -> Result<Vec<OrderedRelease>> {
    let ordered = catalog.install_order(selected)?;
    ordered
        .into_iter()
        .map(|package| {
            let release = catalog.release(&package)?;
            Ok(OrderedRelease { package, release })
        })
        .collect()
}
