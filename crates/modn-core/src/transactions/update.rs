use super::catalog::{CatalogInstall, CatalogRelease};
use super::install::Fault;
use super::journal::{lock_store, ManagedPackage};
use super::uninstall::Dependents;
use super::validation::validate_transaction_root;
use crate::planning::InstallPlan;
use crate::state::Store;
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Transaction};
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Clone, Copy)]
enum Link {
    Dependencies,
    Dependents,
}

#[derive(Clone, Copy)]
pub(super) struct Replacement<'a> {
    pub(super) package_id: &'a str,
    pub(super) journal_id: &'a str,
}

impl Store {
    /// Replace managed package `package_id` with another release of the same
    /// catalog mod, in one recoverable transaction.
    ///
    /// The old release moves aside under a pending uninstall journal. The new
    /// release then installs with the same checks as
    /// [`Store::install_catalog_release_with_permissions`], except that the old
    /// package's ownership does not block it. One SQLite commit retires the
    /// old package, records the new one and moves its dependents' links to the
    /// new package. A failure before that commit restores the old release
    /// under its package ID. After an interruption, [`Store::recover`] does
    /// the same. An update never overwrites a file that Modn does not own.
    pub fn update_catalog_release(
        &mut self,
        game: &Path,
        package_id: &str,
        archive: &Path,
        proposal: &InstallPlan,
        release: &CatalogRelease,
        approved: &[String],
    ) -> Result<ManagedPackage> {
        self.update_inner(game, package_id, archive, proposal, release, approved, None)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "tests inject a fault into the public update"
    )]
    pub(super) fn update_inner(
        &mut self,
        game: &Path,
        package_id: &str,
        archive: &Path,
        proposal: &InstallPlan,
        release: &CatalogRelease,
        approved: &[String],
        fault: Option<Fault>,
    ) -> Result<ManagedPackage> {
        let _guard = lock_store(self)?;
        self.ensure_idle()?;
        let game = validate_transaction_root(game)?;
        let old = self
            .installed(&game)?
            .into_iter()
            .find(|package| package.id == package_id)
            .with_context(|| format!("managed package {package_id} not found for selected game"))?;
        if old.pinned {
            bail!("managed package {package_id} is pinned; run `modn unpin {package_id}` to update it");
        }
        self.check_pinned_dependents(&game, &old, release)?;
        if old.catalog.as_ref() != Some(&release.reference) {
            bail!(
                "managed package {package_id} was not installed from {}",
                release.reference
            );
        }
        if old.archive_sha256 == proposal.archive_sha256 {
            bail!(
                "{} is already installed from this archive",
                release.reference
            );
        }
        let needs = self.ready_dependencies(&game, &release.dependencies)?;
        let needs_old = self.linked(vec![package_id.to_owned()], Link::Dependents)?;
        if let Some(cycle) = self
            .linked(needs, Link::Dependencies)?
            .into_iter()
            .find(|id| needs_old.contains(id))
        {
            bail!(
                "{} would depend on {cycle}, which needs {package_id}: a dependency cycle",
                release.reference
            );
        }
        let journal_id = self.withdraw_locked(&game, package_id, Dependents::Repoint)?;
        let catalog = CatalogInstall {
            release,
            stack_id: None,
            replacing: Some(Replacement {
                package_id,
                journal_id: &journal_id,
            }),
        };
        match self.install_locked(&game, archive, proposal, fault, approved, Some(catalog)) {
            Ok(package) => {
                self.prune_created(&game)
                    .context("update committed; folder cleanup incomplete, run 'modn recover'")?;
                Ok(package)
            }
            Err(err) if matches!(fault, Some(Fault::CrashAfter(_))) => Err(err),
            Err(err) => {
                if let Err(recovery) = self.recover_locked() {
                    return Err(anyhow!(
                        "{err:#}; update rollback incomplete: {recovery:#}; run 'modn recover'"
                    ));
                }
                if self.installed(&game)?.iter().any(|p| p.id == package_id) {
                    Err(err.context(format!("update rolled back; {package_id} stays installed")))
                } else {
                    Err(err.context("update committed; cleanup incomplete, run 'modn recover'"))
                }
            }
        }
    }

    fn linked(&self, start: Vec<String>, link: Link) -> Result<BTreeSet<String>> {
        let query = match link {
            Link::Dependencies => {
                "SELECT dependency_id FROM package_dependencies WHERE package_id=?1"
            }
            Link::Dependents => {
                "SELECT package_id FROM package_dependencies WHERE dependency_id=?1"
            }
        };
        let mut statement = self.connection.prepare(query)?;
        let mut found = BTreeSet::new();
        let mut queue = start;
        while let Some(id) = queue.pop() {
            if !found.insert(id.clone()) {
                continue;
            }
            for next in statement.query_map([&id], |row| row.get::<_, String>(0))? {
                queue.push(next?);
            }
        }
        Ok(found)
    }

    fn check_pinned_dependents(
        &self,
        game: &Path,
        old: &ManagedPackage,
        release: &CatalogRelease,
    ) -> Result<()> {
        for dependent in self.dependents_of(game, &old.id)? {
            if !dependent.pinned {
                continue;
            }
            for requirement in self
                .recorded_dependencies(&dependent.id)?
                .unwrap_or_default()
            {
                if requirement.reference != release.reference {
                    continue;
                }
                if let Some(text) = requirement.requirement {
                    let accepted = release.version.as_deref().is_some_and(|version| {
                        crate::buckets::VersionReq::parse(&text)
                            .ok()
                            .and_then(|req| req.matches(version).ok())
                            .unwrap_or(false)
                    });
                    if !accepted {
                        bail!(
                            "managed package {} is pinned and requires {} {text}",
                            dependent.id,
                            release.reference
                        );
                    }
                }
            }
        }
        Ok(())
    }

    /// Managed packages whose recorded dependencies include `package_id`.
    pub fn dependents_of(&self, game: &Path, package_id: &str) -> Result<Vec<ManagedPackage>> {
        let mut statement = self
            .connection
            .prepare("SELECT package_id FROM package_dependencies WHERE dependency_id=?1")?;
        let ids = statement
            .query_map([package_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(self
            .installed(game)?
            .into_iter()
            .filter(|package| ids.contains(&package.id))
            .collect())
    }
}

pub(super) fn vacate(tx: &Transaction<'_>, replaced: Replacement<'_>) -> Result<()> {
    let package_id = replaced.package_id;
    tx.execute(
        "DELETE FROM managed_directories WHERE package_id=?1",
        [package_id],
    )?;
    tx.execute(
        "DELETE FROM managed_files WHERE package_id=?1",
        [package_id],
    )?;
    super::catalog::forget(tx, package_id)?;
    Ok(())
}

pub(super) fn retire(
    tx: &Transaction<'_>,
    replaced: Replacement<'_>,
    successor: &str,
) -> Result<()> {
    tx.execute(
        "UPDATE package_dependencies SET dependency_id=?2 WHERE dependency_id=?1",
        params![replaced.package_id, successor],
    )?;
    tx.execute(
        "DELETE FROM managed_packages WHERE id=?1",
        [replaced.package_id],
    )?;
    let committed = tx.execute(
        "UPDATE pending_transactions SET status='committed'
         WHERE id=?1 AND action='uninstall' AND status='pending'",
        [replaced.journal_id],
    )?;
    if committed != 1 {
        bail!("the replaced release is no longer withdrawn; run 'modn recover'");
    }
    Ok(())
}
