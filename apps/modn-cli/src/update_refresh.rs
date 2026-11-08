//! Refresh every catalog when `modn update` has no target.
//!
//! The official bucket uses the same signed-release path as `modn bucket update`.
//! Each registered Git catalog uses the same path as `modn bucket sync <name>`.
//! Local folder buckets have nothing to fetch and are reported as skipped.
//! Every step is independent: one failure is reported, the rest still run, and
//! the command exits with a nonzero status.

use crate::bucket_update;
use crate::buckets;
use crate::commands::UpdateArgs;
use anyhow::{bail, Result};
use modn_core::buckets::{BucketStore, GitCatalogSync};
use serde_json::{json, Value};
use std::path::Path;

const LOCAL_SKIP: &str = "local folder bucket has nothing to fetch";

enum Detail {
    Main {
        changed: bool,
        key_id: String,
    },
    Git {
        changed: bool,
        commit: String,
        signer: String,
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
            Ok(Detail::Main { changed, key_id }) => json!({
                "name": self.name,
                "kind": self.kind,
                "changed": changed,
                "signed_by": key_id,
            }),
            Ok(Detail::Git {
                changed,
                commit,
                signer,
            }) => json!({
                "name": self.name,
                "kind": self.kind,
                "changed": changed,
                "commit": commit,
                "signer_key_id": signer,
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
            Ok(Detail::Main { changed, key_id }) => {
                format!("{prefix}: changed: {changed}, signed by {key_id}")
            }
            Ok(Detail::Git {
                changed,
                commit,
                signer,
            }) => format!("{prefix}: changed: {changed}, commit {commit}, signer {signer}"),
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
    Ok(())
}

fn main_refresh() -> Refresh {
    match bucket_update::refresh() {
        Ok(refreshed) => Refresh::main(Ok(Detail::Main {
            changed: refreshed.changed,
            key_id: refreshed.key_id.to_string(),
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
            })
        },
    );
    Refresh {
        name: name.into(),
        kind: "git",
        outcome,
    }
}

fn catalog_refreshes(state_path: &Path, json: bool) -> Vec<Refresh> {
    let mut refreshes = vec![main_refresh()];
    let Some(root) = state_path.parent() else {
        refreshes.push(Refresh {
            name: "registrations".into(),
            kind: "registry",
            outcome: Err("invalid manager state directory".into()),
        });
        return refreshes;
    };
    let store = BucketStore::new(root);
    if let Err(error) = buckets::validate_registry_write(state_path, root, &store, None) {
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
        if info.signer_key_id.is_some() {
            refreshes.push(git_refresh(&store, &info.name, json));
        } else {
            refreshes.push(Refresh {
                name: info.name,
                kind: "local",
                outcome: Ok(Detail::Local),
            });
        }
    }
    refreshes
}

pub(super) fn run(
    args: &UpdateArgs,
    state_path: &Path,
    requested: Option<&str>,
    json: bool,
) -> Result<()> {
    refuse_target_options(args, requested)?;
    let refreshes = catalog_refreshes(state_path, json);
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
