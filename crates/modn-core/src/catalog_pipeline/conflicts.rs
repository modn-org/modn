//! Declared conflicts between an install order and installed packages.
//!
//! Only releases this installation would place are checked. The read uses the
//! verified archive digest and the catalog identity the snapshot pinned. This
//! step never writes.

use super::archives::ResolvedArchives;
use super::order::OrderedRelease;
use super::plan::PlanConflict;
use crate::state::Store;
use anyhow::Result;
use std::path::Path;

/// Find every declared conflict between the installing releases and managed
/// packages of the target installation.
pub(super) fn resolve(
    order: &[OrderedRelease],
    archives: &ResolvedArchives,
    installing: &[usize],
    store: Option<&Store>,
    installation: &Path,
) -> Result<Vec<PlanConflict>> {
    let Some(store) = store else {
        return Ok(Vec::new());
    };
    let mut conflicts = Vec::new();
    for &index in installing {
        let installed = store.conflicting_packages_for_digest(
            installation,
            Some(&order[index].release),
            &archives.releases[index].digest,
        )?;
        if !installed.is_empty() {
            conflicts.push(PlanConflict {
                reference: order[index].package.reference(),
                installed,
            });
        }
    }
    Ok(conflicts)
}
