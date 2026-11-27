use crate::commands::InstallArgs;
use crate::install_consent;
use crate::progress::Progress;
use anyhow::{bail, Context, Result};
use modn_core::buckets::{BucketPackage, BucketSnapshot, BucketStore};
use modn_core::error::{coded, ErrorCode};
use modn_core::manager_roots::ManagerRoots;
use modn_core::transactions::CatalogReference;
use std::fmt;
use std::io::{self, IsTerminal, Write};
#[cfg(test)]
#[path = "catalog_snapshot_tests.rs"]
mod tests;

mod commit;
mod flow;
mod plan_data;

#[derive(Debug)]
pub(super) struct AmbiguousTarget {
    query: String,
    candidates: Vec<String>,
    pub(super) json: bool,
}

impl AmbiguousTarget {
    pub(super) fn as_json(&self) -> serde_json::Value {
        serde_json::json!({
            "error": ErrorCode::AmbiguousTarget.as_str(),
            "query": self.query,
            "candidates": self.candidates,
        })
    }
}

impl fmt::Display for AmbiguousTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ambiguous target {:?}; choose one of: {}",
            self.query,
            self.candidates.join(", ")
        )
    }
}

impl std::error::Error for AmbiguousTarget {}

pub(super) fn resolve(query: &str, catalog: &BucketSnapshot, json: bool) -> Result<BucketPackage> {
    let mut candidates = catalog.candidates(query)?;
    match candidates.len() {
        0 => Err(coded(
            ErrorCode::ModNotFound,
            format!("catalog mod not found: {query}"),
        )),
        1 => Ok(candidates.remove(0)),
        _ if json || !io::stdin().is_terminal() || !io::stderr().is_terminal() => {
            Err(AmbiguousTarget {
                query: query.into(),
                candidates: candidates
                    .iter()
                    .map(|p| format!("{}/{}/{}", p.bucket, p.package.game, p.package.id))
                    .collect(),
                json,
            }
            .into())
        }
        _ => {
            eprintln!("Multiple matches for {query:?}. Which one do you mean?");
            for (index, candidate) in candidates.iter().enumerate() {
                eprintln!(
                    "  {}. {}/{}/{}",
                    index + 1,
                    candidate.bucket,
                    candidate.package.game,
                    candidate.package.id
                );
            }
            eprint!("Select [1-{}]: ", candidates.len());
            io::stderr().flush()?;
            let mut answer = String::new();
            io::stdin().read_line(&mut answer)?;
            let index: usize = answer.trim().parse().context("invalid selection")?;
            if !(1..=candidates.len()).contains(&index) {
                bail!("selection out of range");
            }
            Ok(candidates.remove(index - 1))
        }
    }
}

pub(super) fn reference_of(package: &BucketPackage) -> CatalogReference {
    package.reference()
}

pub(super) fn refuse_unpromptable(interactive: bool, json: bool) -> Result<()> {
    if interactive && !install_consent::can_prompt(json) {
        bail!("installation writes game files; choose --dry-run, confirm in a terminal, or explicitly approve with --yes");
    }
    Ok(())
}

pub(super) fn run(
    args: &InstallArgs,
    roots: &ManagerRoots,
    game_filter: Option<&str>,
    json: bool,
    progress: &Progress,
) -> Result<()> {
    let catalog = BucketStore::new(roots).snapshot(game_filter)?;
    let package = resolve(&args.target, &catalog, json)?;
    flow::install_mod(package, &catalog, args, roots, json, progress)
}
