use crate::bucket_update;
use crate::buckets;
use crate::commands::UpdateArgs;
use crate::main_release::UnsupportedRelease;
use crate::state_access::validate_write_targets;
use anyhow::{bail, Result};
use modn_core::buckets::{BucketStore, CatalogKeyChange, GitCatalogSync, ZipCatalogSync};
use modn_core::manager_roots::ManagerRoots;
use serde_json::{json, Value};

const LOCAL_SKIP: &str = "local folder bucket has nothing to fetch";

enum Detail {
    Main {
        changed: bool,
        key_id: String,
        release: Option<String>,
        newer: Option<UnsupportedRelease>,
    },
    Git {
        changed: bool,
        commit: String,
        signer: String,
        key_change: Option<CatalogKeyChange>,
    },
    Zip {
        changed: bool,
        digest: String,
        signer: String,
        key_change: Option<CatalogKeyChange>,
    },
    Local,
}

struct Refresh {
    name: String,
    kind: &'static str,
    outcome: std::result::Result<Detail, String>,
}

impl Refresh {
    fn main(outcome: std::result::Result<Detail, String>) -> Self {
        Self {
            name: "main".into(),
            kind: "official",
            outcome,
        }
    }

    fn json(&self) -> Value {
        match &self.outcome {
            Ok(Detail::Main {
                changed,
                key_id,
                release,
                newer,
            }) => json!({
                "name": self.name,
                "kind": self.kind,
                "changed": changed,
                "release": release,
                "signed_by": key_id,
                "newer_release_needs_modn": newer.as_ref().map(|newer| &newer.min_modn_version),
            }),
            Ok(Detail::Git {
                changed,
                commit,
                signer,
                key_change,
            }) => json!({
                "name": self.name,
                "kind": self.kind,
                "changed": changed,
                "commit": commit,
                "signer_key_id": signer,
                "key_change": key_change,
            }),
            Ok(Detail::Zip {
                changed,
                digest,
                signer,
                key_change,
            }) => json!({
                "name": self.name,
                "kind": self.kind,
                "changed": changed,
                "digest": digest,
                "signer_key_id": signer,
                "key_change": key_change,
            }),
            Ok(Detail::Local) => json!({
                "name": self.name,
                "kind": self.kind,
                "skipped": LOCAL_SKIP,
            }),
            Err(error) => json!({
                "name": self.name,
                "kind": self.kind,
                "error": error,
            }),
        }
    }

    fn line(&self) -> String {
        let prefix = format!("{} ({})", self.name, self.kind);
        match &self.outcome {
            Ok(Detail::Main {
                changed,
                key_id,
                release,
                newer,
            }) => {
                let release = release
                    .as_ref()
                    .map_or_else(String::new, |tag| format!(", release {tag}"));
                let newer = newer.as_ref().map_or_else(String::new, |newer| {
                    format!(", newer release {} {}", newer.tag, newer.needs())
                });
                format!("{prefix}: changed: {changed}, signed by {key_id}{release}{newer}")
            }
            Ok(Detail::Git {
                changed,
                commit,
                signer,
                key_change,
            }) => format!(
                "{prefix}: changed: {changed}, commit {commit}, signer {signer}{}",
                key_change_suffix(key_change.as_ref())
            ),
            Ok(Detail::Zip {
                changed,
                digest,
                signer,
                key_change,
            }) => format!(
                "{prefix}: changed: {changed}, digest {digest}, signer {signer}{}",
                key_change_suffix(key_change.as_ref())
            ),
            Ok(Detail::Local) => format!("{prefix}: skipped; {LOCAL_SKIP}"),
            Err(error) => format!("{prefix}: error: {error}"),
        }
    }
}

fn refuse_target_options(args: &UpdateArgs, requested: Option<&str>) -> Result<()> {
    if args.version.is_some() {
        bail!("`--version` needs an update target: run `modn update <target> --version <release>`");
    }
    if args.file.is_some() {
        bail!("`--file` needs an update target: run `modn update <target> --file <archive>`");
    }
    if args.path.is_some() {
        bail!("`--path` needs an update target: run `modn update <target> --path <game>`");
    }
    if !args.nxm.is_empty() {
        bail!("`--nxm` needs an update target: run `modn update <target> --nxm <link>`");
    }
    if requested.is_some() {
        bail!("`--game` needs an update target: run `modn --game <id> update <target>`");
    }
    for (given, flag) in [
        (args.dry_run, "--dry-run"),
        (args.no_cache, "--no-cache"),
        (!args.allow_executable.is_empty(), "--allow-executable"),
        (args.allow_native_injector, "--allow-native-injector"),
    ] {
        if given {
            bail!("`{flag}` needs an update target: run `modn update <target> {flag}`");
        }
    }
    Ok(())
}

fn main_refresh() -> Refresh {
    match bucket_update::refresh() {
        Ok(refreshed) => Refresh::main(Ok(Detail::Main {
            changed: refreshed.changed,
            key_id: refreshed.key_id.to_string(),
            release: refreshed.release,
            newer: refreshed.newer,
        })),
        Err(error) => Refresh::main(Err(format!("{error:#}"))),
    }
}

fn git_refresh(store: &BucketStore, name: &str, json: bool) -> Refresh {
    let previous = store
        .git_revision(name)
        .ok()
        .flatten()
        .map(|revision| revision.commit);
    let outcome = buckets::sync_catalog(store, name, None, json).map_or_else(
        |error| Err(format!("{error:#}")),
        |synced: GitCatalogSync| {
            let commit = synced.revision.commit.clone();
            Ok(Detail::Git {
                changed: previous.as_deref() != Some(commit.as_str()),
                commit,
                signer: synced.revision.signer.key_id.clone(),
                key_change: synced.key_change,
            })
        },
    );
    Refresh {
        name: name.into(),
        kind: "git",
        outcome,
    }
}

fn zip_refresh(store: &BucketStore, name: &str, json: bool) -> Refresh {
    let previous = store
        .zip_revision(name)
        .ok()
        .flatten()
        .map(|revision| revision.digest);
    let outcome = buckets::sync_zip_catalog(store, name, None, json).map_or_else(
        |error| Err(format!("{error:#}")),
        |synced: ZipCatalogSync| {
            let digest = synced.revision.digest.clone();
            Ok(Detail::Zip {
                changed: previous.as_deref() != Some(digest.as_str()),
                digest,
                signer: synced.revision.signer.key_id.clone(),
                key_change: synced.key_change,
            })
        },
    );
    Refresh {
        name: name.into(),
        kind: "zip",
        outcome,
    }
}
fn key_change_suffix(change: Option<&CatalogKeyChange>) -> String {
    change.map_or_else(String::new, |change| format!(", {}", change.line()))
}
fn catalog_refreshes(roots: &ManagerRoots, json: bool) -> Vec<Refresh> {
    let mut refreshes = vec![main_refresh()];
    let root = roots.profile();
    let store = BucketStore::new(root);
    if let Err(error) = validate_write_targets(roots, None) {
        refreshes.push(Refresh {
            name: "registrations".into(),
            kind: "registry",
            outcome: Err(format!("{error:#}")),
        });
        return refreshes;
    }
    let listed = match store.list() {
        Ok(listed) => listed,
        Err(error) => {
            refreshes.push(Refresh {
                name: "registrations".into(),
                kind: "registry",
                outcome: Err(format!("{error:#}")),
            });
            return refreshes;
        }
    };
    for info in listed.into_iter().filter(|info| info.name != "main") {
        match info.source_kind.as_deref() {
            Some("zip") => refreshes.push(zip_refresh(&store, &info.name, json)),
            Some("git") => refreshes.push(git_refresh(&store, &info.name, json)),
            _ => refreshes.push(Refresh {
                name: info.name,
                kind: "local",
                outcome: Ok(Detail::Local),
            }),
        }
    }
    refreshes
}

pub(super) fn run(
    args: &UpdateArgs,
    roots: &ManagerRoots,
    requested: Option<&str>,
    json: bool,
) -> Result<()> {
    refuse_target_options(args, requested)?;
    let refreshes = catalog_refreshes(roots, json);
    let failed = refreshes
        .iter()
        .filter(|refresh| refresh.outcome.is_err())
        .count();
    if json {
        let buckets: Vec<Value> = refreshes.iter().map(Refresh::json).collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"buckets": buckets, "failed": failed}))?
        );
    } else {
        for refresh in &refreshes {
            println!("{}", refresh.line());
        }
    }
    if failed > 0 {
        bail!("{failed} of {} catalog refreshes failed", refreshes.len());
    }
    Ok(())
}
