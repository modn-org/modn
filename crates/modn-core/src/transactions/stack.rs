use super::catalog::{CatalogInstall, CatalogRelease};
use super::journal::{lock_store, ManagedPackage};
use super::native_injector::NativeInjectorPolicy;
use crate::planning::InstallPlan;
use crate::state::Store;
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::params;
use std::path::{Path, PathBuf};
use tempfile::Builder;

/// One stack member: a verified archive, its catalog release and approvals.
#[expect(missing_docs, reason = "Field and variant names state their meaning")]
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct StackMember {
    /// Archive whose digest the caller verified against the release manifest.
    pub archive: PathBuf,
    pub release: CatalogRelease,
    /// Tool executables the user approved for this member.
    pub approved_executables: Vec<String>,
}

#[expect(missing_docs, reason = "Method names state their behavior")]
impl StackMember {
    #[must_use]
    pub fn new(
        archive: PathBuf,
        release: CatalogRelease,
        approved_executables: Vec<String>,
    ) -> Self {
        Self {
            archive,
            release,
            approved_executables,
        }
    }
}

impl Store {
    /// Install `members` in order, as one unit, under one transaction lock.
    ///
    /// `plan` proposes each member's operations just before it is installed,
    /// so game policy sees the files of earlier members. Each proposal is
    /// validated as for a single catalog install, and each member's
    /// dependencies must be ready when its turn comes. Any failure uninstalls
    /// the members already installed by this stack, newest first.
    pub fn install_catalog_stack(
        &mut self,
        game: &Path,
        members: &[StackMember],
        policy: NativeInjectorPolicy,
        mut plan: impl FnMut(&StackMember) -> Result<InstallPlan>,
    ) -> Result<Vec<ManagedPackage>> {
        for member in members {
            member.release.ensure_native_injector_allowed(policy)?;
        }
        let _guard = lock_store(self)?;
        self.ensure_idle()?;
        let game = self.transaction_root(game)?;
        if members.is_empty() {
            bail!("installation stack is empty");
        }
        self.control.checkpoint()?;
        let stack_id = self.begin_stack(&game)?;
        let mut installed = Vec::new();
        for member in members {
            let reference = &member.release.reference;
            let result = plan(member).and_then(|proposal| {
                self.install_locked(
                    &game,
                    &member.archive,
                    &proposal,
                    None,
                    &member.approved_executables,
                    Some(CatalogInstall {
                        release: &member.release,
                        stack_id: Some(&stack_id),
                        replacing: None,
                    }),
                    policy,
                )
            });
            match result {
                Ok(package) => installed.push(package),
                Err(err) => {
                    let err = err.context(format!("{reference}: installation stack failed"));
                    return Err(match self.rollback_stack(&stack_id, &game) {
                        Ok(count) => err.context(format!(
                            "rolled back {count} package(s) installed by this stack"
                        )),
                        Err(rollback) => anyhow!(
                            "{err:#}; stack rollback incomplete: {rollback:#}; run 'modn recover'"
                        ),
                    });
                }
            }
        }
        self.finish_stack(&stack_id, Some(&installed)).context(
            "installation stack not finalized; 'modn recover' rolls back all of its packages",
        )?;
        Ok(installed)
    }

    pub(super) fn ensure_idle(&self) -> Result<()> {
        self.ensure_no_pending()?;
        let stacks: i64 =
            self.connection
                .query_row("SELECT COUNT(*) FROM pending_stacks", [], |row| row.get(0))?;
        if stacks > 0 {
            bail!(
                "unfinished installation stack detected; run 'modn recover' before another write"
            );
        }
        Ok(())
    }

    pub(super) fn recover_stacks_locked(&mut self) -> Result<usize> {
        let mut statement = self
            .connection
            .prepare("SELECT id, game_path FROM pending_stacks ORDER BY id")?;
        let stacks = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        for (id, game) in &stacks {
            self.rollback_stack(id, Path::new(game))?;
        }
        Ok(stacks.len())
    }

    pub(super) fn begin_stack(&mut self, game: &Path) -> Result<String> {
        let marker = Builder::new()
            .prefix("stack-")
            .tempdir_in(&self.state_root)?;
        let id = marker
            .path()
            .file_name()
            .context("missing stack id")?
            .to_string_lossy()
            .into_owned();
        self.connection.execute(
            "INSERT INTO pending_stacks(id,game_path) VALUES (?1,?2)",
            params![id, game.to_string_lossy().as_ref()],
        )?;
        Ok(id)
    }

    fn finish_stack(&mut self, id: &str, installed: Option<&[ManagedPackage]>) -> Result<()> {
        let record = match (self.history.as_ref(), installed) {
            (Some(context), Some(packages)) => {
                Some(crate::history::stack_operation(context, id, packages))
            }
            _ => None,
        };
        let tx = self.connection.transaction()?;
        if let Some(record) = &record {
            crate::history::insert(&tx, record)?;
        }
        tx.execute("DELETE FROM pending_stack_packages WHERE stack_id=?1", [id])?;
        tx.execute("DELETE FROM pending_stacks WHERE id=?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    fn rollback_stack(&mut self, id: &str, game: &Path) -> Result<usize> {
        let context = self.history.take();
        let result = self.rollback_stack_members(id, game);
        self.history = context;
        result
    }

    fn rollback_stack_members(&mut self, id: &str, game: &Path) -> Result<usize> {
        let mut statement = self.connection.prepare(
            "SELECT package_id FROM pending_stack_packages WHERE stack_id=?1 ORDER BY ord DESC",
        )?;
        let members = statement
            .query_map([id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        for package_id in &members {
            self.uninstall_locked(game, package_id)
                .with_context(|| format!("cannot roll back stack package {package_id}"))?;
        }
        self.finish_stack(id, None)?;
        Ok(members.len())
    }
}
