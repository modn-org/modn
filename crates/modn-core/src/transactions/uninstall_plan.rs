//! Uninstall by name, with optional removal of dependencies and dependents.
//!
//! A selector is a managed-package ID, or the catalog name the package was
//! installed under: `mod`, `game/mod` or `bucket/game/mod`. The plan lists
//! every package to remove, dependents first, so each removal keeps the
//! rule that a package is never removed while another one needs it.
use super::journal::{lock_store, ManagedPackage};
use super::validation::validate_transaction_root;
use super::CatalogReference;
use crate::state::Store;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Which related packages leave together with the selected package.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct UninstallScope {
    /// Also remove each dependency that no remaining package needs.
    pub dependencies: bool,
    /// Also remove every package that needs the selected package.
    pub dependents: bool,
}

impl UninstallScope {
    /// Also remove each dependency that no remaining package needs, when `include` is set.
    #[must_use]
    pub fn including_dependencies(mut self, include: bool) -> Self {
        self.dependencies = include;
        self
    }

    /// Also remove every package that needs the selection, when `include` is set.
    #[must_use]
    pub fn including_dependents(mut self, include: bool) -> Self {
        self.dependents = include;
        self
    }
}

/// One package an uninstall removes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct PlannedRemoval {
    /// Managed package ID.
    pub id: String,
    /// Catalog identity, when the package was installed from a bucket.
    pub catalog: Option<CatalogReference>,
    /// Number of files the package owns.
    pub files: usize,
}

/// Dependency links between the managed packages of one game.
struct Links {
    /// Package to the packages it needs.
    needs: BTreeMap<String, BTreeSet<String>>,
    /// Package to the packages that need it.
    needed_by: BTreeMap<String, BTreeSet<String>>,
}

impl Links {
    fn load(store: &Store, packages: &[ManagedPackage]) -> Result<Self> {
        let known: BTreeSet<&str> = packages.iter().map(|p| p.id.as_str()).collect();
        let mut links = Self {
            needs: BTreeMap::new(),
            needed_by: BTreeMap::new(),
        };
        if !store.has_table("package_dependencies")? {
            return Ok(links);
        }
        let mut statement = store
            .connection
            .prepare("SELECT package_id, dependency_id FROM package_dependencies")?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (package, dependency) in rows {
            if known.contains(package.as_str()) && known.contains(dependency.as_str()) {
                links
                    .needs
                    .entry(package.clone())
                    .or_default()
                    .insert(dependency.clone());
                links
                    .needed_by
                    .entry(dependency)
                    .or_default()
                    .insert(package);
            }
        }
        Ok(links)
    }

    fn needing(&self, id: &str) -> impl Iterator<Item = &String> {
        self.needed_by.get(id).into_iter().flatten()
    }

    fn needed(&self, id: &str) -> impl Iterator<Item = &String> {
        self.needs.get(id).into_iter().flatten()
    }
}

impl Store {
    /// Plan an uninstall without changing anything.
    ///
    /// The plan lists packages in removal order. It fails when `selector`
    /// matches no package or several, or when other packages need the
    /// selection and `scope` does not include them.
    pub fn plan_uninstall(
        &self,
        game: &Path,
        selector: &str,
        scope: UninstallScope,
    ) -> Result<Vec<PlannedRemoval>> {
        let packages = self.installed(game)?;
        let links = Links::load(self, &packages)?;
        let target = resolve(&packages, selector)?;
        let label = |id: &str| -> String {
            packages
                .iter()
                .find(|p| p.id == id)
                .and_then(|p| p.catalog.as_ref())
                .map_or_else(|| id.to_owned(), |c| format!("{id} ({c})"))
        };

        let mut removal = BTreeSet::from([target.clone()]);
        let mut queue = vec![target.clone()];
        while let Some(id) = queue.pop() {
            for dependent in links.needing(&id) {
                if removal.insert(dependent.clone()) {
                    queue.push(dependent.clone());
                }
            }
        }
        if removal.len() > 1 && !scope.dependents {
            let others: Vec<String> = removal
                .iter()
                .filter(|id| **id != target)
                .map(|id| label(id))
                .collect();
            bail!(
                "managed package {target} is required by {}; uninstall dependents first or remove them together",
                others.join(", ")
            );
        }

        if scope.dependencies {
            let mut changed = true;
            while changed {
                changed = false;
                let members: Vec<String> = removal.iter().cloned().collect();
                for member in members {
                    for dependency in links.needed(&member) {
                        let free = links.needing(dependency).all(|d| removal.contains(d));
                        if free && removal.insert(dependency.clone()) {
                            changed = true;
                        }
                    }
                }
            }
        }

        // Newest first, and never a package before the ones that need it.
        let mut order = Vec::new();
        while !removal.is_empty() {
            let next = packages
                .iter()
                .rev()
                .map(|p| &p.id)
                .find(|id| removal.contains(*id) && links.needing(id).all(|d| !removal.contains(d)))
                .context("dependency cycle among managed packages")?
                .clone();
            removal.remove(&next);
            order.push(next);
        }
        Ok(order
            .into_iter()
            .filter_map(|id| packages.iter().find(|p| p.id == id))
            .map(|p| PlannedRemoval {
                id: p.id.clone(),
                catalog: p.catalog.clone(),
                files: p.files.len(),
            })
            .collect())
    }

    /// Uninstall the packages `plan_uninstall` lists, one verified removal at
    /// a time, under one transaction lock.
    ///
    /// Each removal is its own recoverable transaction. When one fails, the
    /// packages removed before it stay removed, the rest stay installed, and
    /// the error names both.
    pub fn uninstall_with(
        &mut self,
        game: &Path,
        selector: &str,
        scope: UninstallScope,
    ) -> Result<Vec<String>> {
        let _guard = lock_store(self)?;
        self.ensure_idle()?;
        let plan = self.plan_uninstall(game, selector, scope)?;
        let game = validate_transaction_root(game)?;
        let mut removed: Vec<String> = Vec::new();
        for step in &plan {
            self.uninstall_locked(&game, &step.id).with_context(|| {
                format!(
                    "stopped at {} after removing [{}]; the remaining packages are still installed",
                    step.id,
                    removed.join(", ")
                )
            })?;
            removed.push(step.id.clone());
        }
        Ok(removed)
    }
}

/// The managed package an ID or catalog name selects.
fn resolve(packages: &[ManagedPackage], selector: &str) -> Result<String> {
    if let Some(package) = packages.iter().find(|p| p.id == selector) {
        return Ok(package.id.clone());
    }
    let matches: Vec<&ManagedPackage> = packages
        .iter()
        .filter(|p| p.catalog.as_ref().is_some_and(|c| names(c, selector)))
        .collect();
    match matches.as_slice() {
        [] => bail!("no managed package matches {selector}; run 'modn list'"),
        [package] => Ok(package.id.clone()),
        several => bail!(
            "{selector} matches several managed packages: {}; use an ID or a qualified name",
            several
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Whether `selector` is `mod`, `game/mod` or `bucket/game/mod` of `catalog`.
fn names(catalog: &CatalogReference, selector: &str) -> bool {
    selector == catalog.mod_id
        || selector == format!("{}/{}", catalog.game_id, catalog.mod_id)
        || selector == catalog.to_string()
}
