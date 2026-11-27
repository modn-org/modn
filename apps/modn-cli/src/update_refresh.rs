use crate::bucket_update;
use crate::buckets;
use crate::buckets::CatalogReport;
use crate::commands::UpdateArgs;
use crate::main_release::UnsupportedRelease;
use crate::state_access::validate_write_targets;
use anyhow::{bail, Result};
use modn_core::buckets::{BucketStore, CatalogKeyChange};
use modn_core::error::code_of;
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
    report: CatalogReport,
    code: Option<String>,
}

impl Refresh {
    fn new(name: &str, kind: &'static str, outcome: std::result::Result<Detail, String>) -> Self {
        Self {
            name: name.to_owned(),
            kind,
            outcome,
            report: CatalogReport::default(),
            code: None,
        }
    }

    fn main(outcome: std::result::Result<Detail, String>, report: CatalogReport) -> Self {
        let mut refresh = Self::new("main", "official", outcome);
        refresh.report = report;
        refresh
    }

    fn json(&self) -> Value {
        let mut value = match &self.outcome {
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
        };
        if let Value::Object(map) = &mut value {
            map.insert("sequence".to_owned(), json!(self.report.sequence));
            map.insert("warnings".to_owned(), json!(&self.report.warnings));
            if let Some(code) = &self.code {
                map.insert("code".to_owned(), json!(code));
            }
        }
        value
    }

    fn line(&self) -> String {
        let prefix = format!("{} ({})", self.name, self.kind);
        let line = match &self.outcome {
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
            Err(error) => match &self.code {
                Some(code) => format!("{prefix}: error: {error} ({code})"),
                None => format!("{prefix}: error: {error}"),
            },
        };
        line
    }

    fn lines(&self) -> Vec<String> {
        let mut lines = vec![self.line()];
        lines.extend(self.report.lines.iter().cloned());
        lines
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

fn main_refresh(roots: &ManagerRoots) -> Refresh {
    match bucket_update::refresh(roots) {
        Ok(refreshed) => Refresh::main(
            Ok(Detail::Main {
                changed: refreshed.changed,
                key_id: refreshed.key_id.to_string(),
                release: refreshed.release,
                newer: refreshed.newer,
            }),
            refreshed.report,
        ),
        Err(error) => {
            let mut refresh = Refresh::main(Err(format!("{error:#}")), CatalogReport::default());
            refresh.code = code_of(&error).map(|code| code.to_string());
            refresh
        }
    }
}

fn git_refresh(store: &BucketStore, roots: &ManagerRoots, name: &str, json: bool) -> Refresh {
    let previous = store
        .git_revision(name)
        .ok()
        .flatten()
        .map(|revision| revision.commit);
    match buckets::sync_catalog(store, roots, name, None, None, json) {
        Ok(synced) => {
            let report = buckets::catalog_report(roots, name, &synced.warnings);
            let commit = synced.revision.commit.clone();
            let mut refresh = Refresh::new(
                name,
                "git",
                Ok(Detail::Git {
                    changed: previous.as_deref() != Some(commit.as_str()),
                    commit,
                    signer: synced.revision.signer.key_id.clone(),
                    key_change: synced.key_change,
                }),
            );
            refresh.report = report;
            refresh
        }
        Err(error) => {
            let mut refresh = Refresh::new(name, "git", Err(format!("{error:#}")));
            refresh.code = code_of(&error).map(|code| code.to_string());
            refresh
        }
    }
}

fn zip_refresh(store: &BucketStore, roots: &ManagerRoots, name: &str, json: bool) -> Refresh {
    let previous = store
        .zip_revision(name)
        .ok()
        .flatten()
        .map(|revision| revision.digest);
    match buckets::sync_zip_catalog(store, roots, name, None, None, json) {
        Ok(synced) => {
            let report = buckets::catalog_report(roots, name, &synced.warnings);
            let digest = synced.revision.digest.clone();
            let mut refresh = Refresh::new(
                name,
                "zip",
                Ok(Detail::Zip {
                    changed: previous.as_deref() != Some(digest.as_str()),
                    digest,
                    signer: synced.revision.signer.key_id.clone(),
                    key_change: synced.key_change,
                }),
            );
            refresh.report = report;
            refresh
        }
        Err(error) => {
            let mut refresh = Refresh::new(name, "zip", Err(format!("{error:#}")));
            refresh.code = code_of(&error).map(|code| code.to_string());
            refresh
        }
    }
}
fn key_change_suffix(change: Option<&CatalogKeyChange>) -> String {
    change.map_or_else(String::new, |change| format!(", {}", change.line()))
}
fn catalog_refreshes(roots: &ManagerRoots, json: bool) -> Vec<Refresh> {
    let mut refreshes = vec![main_refresh(roots)];
    let store = BucketStore::new(roots);
    if let Err(error) = validate_write_targets(roots, None) {
        let mut refresh = Refresh::new("registrations", "registry", Err(format!("{error:#}")));
        refresh.code = code_of(&error).map(|code| code.to_string());
        refreshes.push(refresh);
        return refreshes;
    }
    let listed = match store.list() {
        Ok(listed) => listed,
        Err(error) => {
            let mut refresh = Refresh::new("registrations", "registry", Err(format!("{error:#}")));
            refresh.code = code_of(&error).map(|code| code.to_string());
            refreshes.push(refresh);
            return refreshes;
        }
    };
    for info in listed.into_iter().filter(|info| info.name != "main") {
        match info.source_kind.as_deref() {
            Some("zip") => refreshes.push(zip_refresh(&store, roots, &info.name, json)),
            Some("git") => refreshes.push(git_refresh(&store, roots, &info.name, json)),
            _ => refreshes.push(Refresh::new(&info.name, "local", Ok(Detail::Local))),
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
            for line in refresh.lines() {
                println!("{line}");
            }
        }
    }
    if failed > 0 {
        bail!("{failed} of {} catalog refreshes failed", refreshes.len());
    }
    Ok(())
}
