//! `modn bucket known`: print the community buckets that `main` lists.
//!
//! The command reads the installed official `main` copy only. It never
//! fetches, registers or trusts a listed bucket.

use anyhow::Result;
use modn_core::buckets::BucketStore;
use modn_core::manager_roots::ManagerRoots;

/// Print the known buckets of the local `main` copy.
pub(super) fn run(roots: &ManagerRoots, json_output: bool) -> Result<()> {
    let store = BucketStore::new(roots);
    let known = store.known_buckets()?;
    let main_installed = main_installed(roots);
    if json_output {
        let buckets: Vec<serde_json::Value> = known
            .entries()
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "name": entry.name,
                    "kind": entry.kind.as_str(),
                    "source": entry.source,
                    "revision": entry.revision,
                    "description": entry.description,
                    "fingerprint": entry.fingerprint.to_string(),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "buckets": buckets }))?
        );
        if known.is_empty() && !main_installed {
            eprintln!("Run `modn bucket update` to fetch the official main bucket.");
        }
        return Ok(());
    }
    if known.is_empty() {
        println!("main lists no known buckets");
        if !main_installed {
            println!("Run `modn bucket update` to fetch the official main bucket.");
        }
        return Ok(());
    }
    for entry in known.entries() {
        println!(
            "{}  {}  {}  {}",
            entry.name,
            entry.kind.as_str(),
            entry.fingerprint,
            entry.source
        );
        println!("  {}", entry.description);
    }
    println!("Sync one with: modn bucket sync <name>");
    Ok(())
}

/// True when the selected profile holds the official `main` bucket.
fn main_installed(roots: &ManagerRoots) -> bool {
    roots.official_main().join("bucket.json").is_file()
}
