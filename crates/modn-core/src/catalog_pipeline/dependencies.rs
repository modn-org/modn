//! Resolve the installed state of every dependency in an install order.
//!
//! The read uses the same digest and integrity rules as installation: a
//! dependency is ready only when a managed package with the pinned digest is
//! intact in the target installation. This step never writes.

use super::archives::ResolvedArchives;
use super::order::OrderedRelease;
use crate::state::Store;
use crate::transactions::{CatalogDependency, DependencyState};
use anyhow::Result;
use std::path::Path;

/// The dependency states of one install order and the releases it would place.
#[derive(Debug)]
pub(super) struct ResolvedDependencies {
    /// One entry per release in the order; `None` for the selected release.
    pub(super) states: Vec<Option<DependencyState>>,
    /// Positions in the order that this installation would place, selected
    /// last. Missing dependencies install; ready, damaged and other-release
    /// dependencies do not.
    pub(super) installing: Vec<usize>,
}

/// Read the state of every dependency and collect the installing set.
///
/// Without a store every dependency is missing, as in a fresh profile.
pub(super) fn resolve(
    order: &[OrderedRelease],
    archives: &ResolvedArchives,
    store: Option<&Store>,
    installation: &Path,
) -> Result<ResolvedDependencies> {
    let (_, dependencies) = order
        .split_last()
        .expect("install order holds the selected release");
    let required: Vec<CatalogDependency> = dependencies
        .iter()
        .zip(&archives.releases)
        .map(|(ordered, archive)| {
            CatalogDependency::with_digest(ordered.package.reference(), archive.digest.clone())
        })
        .collect();
    let states = match store {
        Some(store) => store.dependency_states(installation, &required)?,
        None => vec![DependencyState::Missing; required.len()],
    };
    let mut installing: Vec<usize> = states
        .iter()
        .enumerate()
        .filter(|(_, state)| matches!(state, DependencyState::Missing))
        .map(|(index, _)| index)
        .collect();
    installing.push(dependencies.len());
    Ok(ResolvedDependencies {
        states: states.into_iter().map(Some).chain([None]).collect(),
        installing,
    })
}
