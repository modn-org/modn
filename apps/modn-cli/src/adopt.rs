//! CLI for explicit, per-package adoption confirmation.
use anyhow::{bail, Context, Result};
use modn_core::adoption::preview_adoption_for;
use modn_core::state::Store;
use std::path::Path;

pub(super) fn run(
    store: Option<&mut Store>,
    game_id: &str,
    game: &Path,
    selector: Option<&str>,
    yes: bool,
    dry_run: bool,
    json: bool,
) -> Result<()> {
    if !yes && !dry_run {
        bail!("adoption grants permission to delete manual files on later uninstall; choose --dry-run or explicitly confirm with --yes");
    }
    if yes && selector.is_none() {
        bail!(
            "adoption requires one explicit package selector with --yes; run adopt --dry-run first"
        );
    }
    let mut preview = preview_adoption_for(store.as_deref(), game_id, game)?;
    if let Some(selector) = selector {
        preview
            .packages
            .retain(|package| package.selector == selector);
        if preview.packages.is_empty() {
            bail!("selected manual package not found or already managed: {selector}");
        }
    }
    if dry_run {
        if json {
            println!("{}", serde_json::to_string_pretty(&preview)?);
        } else {
            println!("{}", preview.consequence);
            for package in preview.packages {
                println!("{} [{}]", package.selector, package.kind);
                for file in package.files {
                    println!(
                        "  {} ({} bytes, sha256 {})",
                        file.destination, file.bytes, file.sha256
                    );
                }
            }
        }
    } else {
        let package = preview
            .packages
            .into_iter()
            .next()
            .context("no eligible manual files were selected")?;
        let installed = store
            .context("writable state required for approved adoption")?
            .adopt(game_id, game, &package)?;
        if json {
            println!("{}", serde_json::to_string_pretty(&installed)?);
        } else {
            println!(
                "Adopted {} ({} existing files; later uninstall may delete them)",
                installed.id,
                installed.files.len()
            );
        }
    }
    Ok(())
}
