//! Reconcile on-disk discovery against manager-owned SQLite records without writing to a game.
use crate::buckets::BucketStore;
use crate::content_root::{self, ContentRootHealth, ContentRootStatus};
use crate::game_modules::{hidden_modules, ModuleRegistry};
use crate::integrity::{self, FileRoots, PathState};
use crate::scan::{self, Diagnostic, DoctorReport, IntegrityStatus, ScanReport, ScanRoot};
use crate::state::{recorded_root, RootKind, Store};
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

use crate::transactions::ownership_key;

/// Reconcile owned files using the selected game's ordinary Lua scan policy.
pub fn scan_with_ownership_for(
    store: &Store,
    game_id: &str,
    installation: &Path,
) -> Result<ScanReport> {
    scan_with_ownership_appdata(store, game_id, installation, None)
}

/// Reconcile owned files with an explicit appdata base.
///
/// The explicit base keeps tests and embedding callers away from the real
/// user appdata directory. `None` uses the platform base when the module
/// declares a content root.
pub fn scan_with_ownership_appdata(
    store: &Store,
    game_id: &str,
    installation: &Path,
    appdata: Option<&Path>,
) -> Result<ScanReport> {
    Ok(scan_inner(store, game_id, installation, appdata)?.0)
}

fn scan_inner(
    store: &Store,
    game_id: &str,
    installation: &Path,
    appdata: Option<&Path>,
) -> Result<(ScanReport, Option<ContentRootHealth>)> {
    let game = crate::discovery::validate_installation_for(game_id, installation)?;
    let installed = store.installed(&game)?;
    let health = content_health(store, game_id, &game, appdata)?;
    let content = health.as_ref().and_then(|health| health.root.as_deref());
    let mut report = crate::game_modules::policy::scan(game_id, &game, content, 20_000)?;
    if report.content_root.is_none() {
        report.content_root = health.as_ref().map(|health| health.path.clone());
    }
    let content_unavailable = match health.as_ref().map(|health| health.status) {
        Some(ContentRootStatus::Missing) | None => IntegrityStatus::Missing,
        Some(_) => IntegrityStatus::Unsafe,
    };
    let roots = FileRoots {
        game: &game,
        content,
        content_unavailable,
    };
    let (owned_files, directory_roots) = integrity::owned_file_checks(store, &roots, installed);
    report.owned_files.extend(owned_files);
    report.owned_files.sort_by(|a, b| {
        a.root
            .order()
            .cmp(&b.root.order())
            .then(a.relative_path.cmp(&b.relative_path))
            .then(a.package_id.cmp(&b.package_id))
    });
    let mut matched: HashMap<(ScanRoot, String), Vec<usize>> = HashMap::new();
    for (index, file) in report.owned_files.iter().enumerate() {
        matched
            .entry((file.root, ownership_key(&file.relative_path)))
            .or_default()
            .push(index);
    }

    let mut candidates = vec![Vec::<usize>::new(); report.owned_files.len()];
    for (detected_index, detected) in report.mods.iter().enumerate() {
        let relative = detected.relative_path.to_string_lossy().replace('\\', "/");
        let Some(indices) = matched.get(&(detected.root, ownership_key(&relative))) else {
            continue;
        };
        let Some(base) = scan_root_path(detected.root, &game, content) else {
            continue;
        };
        for &owned_index in indices {
            let owned = &report.owned_files[owned_index];
            if matches!(
                owned.status,
                IntegrityStatus::Missing | IntegrityStatus::Unsafe
            ) {
                continue;
            }
            if owned.status == IntegrityStatus::Unreadable {
                if relative == owned.relative_path {
                    candidates[owned_index].push(detected_index);
                }
            } else if same_file::is_same_file(
                base.join(&owned.relative_path),
                base.join(&detected.relative_path),
            )
            .unwrap_or(false)
            {
                candidates[owned_index].push(detected_index);
            }
        }
    }

    for (owned_index, entries) in candidates.iter().enumerate() {
        let owned = &report.owned_files[owned_index];
        let exact = entries.iter().copied().find(|&index| {
            report.mods[index]
                .relative_path
                .to_string_lossy()
                .replace('\\', "/")
                == owned.relative_path
        });
        let selected = exact.or_else(|| (entries.len() == 1).then(|| entries[0]));
        if let Some(detected_index) = selected {
            let detected = &mut report.mods[detected_index];
            if detected.managed {
                continue;
            }
            detected.managed = true;
            detected.package_id = Some(owned.package_id.clone());
            detected.integrity = Some(owned.status);
        }
    }
    for detected in &mut report.mods {
        if detected.managed || detected.root != ScanRoot::Game {
            continue;
        }
        let relative = detected.relative_path.to_string_lossy().replace('\\', "/");
        let candidate = directory_roots.iter().find(|check| {
            let root = &check.root;
            if check.status != IntegrityStatus::Present
                || !crate::transactions::directory_candidate_key(&relative).starts_with(&format!(
                    "{}/",
                    crate::transactions::directory_candidate_key(root)
                ))
            {
                return false;
            }
            // Folded names are candidates, not proof: NTFS may distinguish
            // Unicode lookalikes even when Rust's lowercase mapping does not.
            let prefix = relative
                .split('/')
                .take(root.split('/').count())
                .collect::<Vec<_>>()
                .join("/");
            same_file::is_same_file(game.join(root), game.join(prefix)).unwrap_or(false)
        });
        if let Some(check) = candidate {
            if integrity::checked_path(&game, &relative, false).0 == PathState::Present {
                detected.managed = true;
                detected.package_id = Some(check.package_id.clone());
                detected.integrity = Some(IntegrityStatus::Present);
            }
        }
    }
    Ok((report, health))
}

fn scan_root_path<'a>(
    root: ScanRoot,
    game: &'a Path,
    content: Option<&'a Path>,
) -> Option<&'a Path> {
    match root {
        ScanRoot::Game => Some(game),
        ScanRoot::Content => content,
    }
}

fn content_health(
    store: &Store,
    game_id: &str,
    game: &Path,
    appdata: Option<&Path>,
) -> Result<Option<ContentRootHealth>> {
    let Some(spec) = crate::game_modules::content_root_spec(game_id)? else {
        return Ok(None);
    };
    let appdata = match appdata {
        Some(appdata) => appdata.to_path_buf(),
        None => content_root::appdata_directory()?,
    };
    let recorded = recorded_root(&store.connection, game, RootKind::Content)?;
    Ok(Some(content_root::inspect(
        game,
        &spec,
        &appdata,
        recorded.as_deref(),
    )))
}

/// Evaluate the selected game's Lua doctor policy over the reconciled inventory.
pub fn doctor_with_ownership_for(
    store: &Store,
    game_id: &str,
    installation: &Path,
) -> Result<DoctorReport> {
    doctor_with_ownership_appdata(store, game_id, installation, None)
}

/// Evaluate diagnostics with an explicit appdata base for the content root.
pub fn doctor_with_ownership_appdata(
    store: &Store,
    game_id: &str,
    installation: &Path,
    appdata: Option<&Path>,
) -> Result<DoctorReport> {
    let source_diagnostics = module_source_diagnostics(game_id)?;
    if source_diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "MODULE_SOURCE_UNTRUSTED")
    {
        let installation = installation.canonicalize()?;
        let created_directories = store.created_directories(&installation)?;
        return Ok(DoctorReport {
            scan: ScanReport {
                incomplete: true,
                scan_issues: vec!["MODULE_SOURCE_UNTRUSTED".into()],
                game_id: game_id.to_owned(),
                installation,
                content_root: None,
                redmod_available: false,
                mods: Vec::new(),
                owned_files: Vec::new(),
                framework_signals: Vec::new(),
            },
            diagnostics: source_diagnostics,
            created_directories,
        });
    }
    let (report, health) = scan_inner(store, game_id, installation, appdata)?;
    let mut report = scan::diagnose_checked(report)?;
    report
        .diagnostics
        .extend(scan::content_root_diagnostics(health.as_ref()));
    let missing = report
        .scan
        .owned_files
        .iter()
        .filter(|file| file.root == ScanRoot::Content && file.status == IntegrityStatus::Missing)
        .count();
    if missing > 0 {
        report.diagnostics.push(Diagnostic {
            code: "CONTENT_ROOT_OWNED_FILE_MISSING".into(),
            severity: "warning".into(),
            message: format!(
                "{missing} manager-owned file(s) are missing from the module content root."
            ),
            action: Some(
                "Restore the original package files from the installation archive, or uninstall \
                 the package, then run `modn doctor` again."
                    .into(),
            ),
        });
    }
    report.diagnostics.extend(source_diagnostics);
    report.created_directories = store.created_directories(installation)?;
    Ok(report)
}

fn module_source_diagnostics(game_id: &str) -> Result<Vec<Diagnostic>> {
    let data = crate::data_dir::data_directory()?;
    let buckets = BucketStore::new(&data);
    let trusted = buckets.trusted_module_sources()?;
    let mut diagnostics = Vec::new();
    if let Some(release) = ModuleRegistry::new(&data).active(game_id)? {
        if let Some(bucket) = release.source.bucket.as_deref() {
            let trusted = bucket == "main"
                || trusted.iter().any(|source| {
                    source.bucket == bucket
                        && Some(&source.signer_key_id) == release.source.signer_key_id.as_ref()
                });
            if !trusted {
                diagnostics.push(Diagnostic {
                    code: "MODULE_SOURCE_UNTRUSTED".into(),
                    severity: "warning".into(),
                    message: format!(
                        "game module {game_id} comes from bucket {bucket}, which is not a trusted module source."
                    ),
                    action: Some(format!(
                        "Run `modn bucket trust-modules {bucket}` to trust it again, or `modn module remove {game_id}` to remove the module."
                    )),
                });
            }
        }
    }
    for hidden in hidden_modules(game_id)? {
        diagnostics.push(Diagnostic {
            code: "MODULE_HIDDEN_BY_OFFICIAL".into(),
            severity: "info".into(),
            message: format!(
                "bucket {} provides game module {game_id} version {}, but the official main bucket takes priority.",
                hidden.bucket, hidden.version
            ),
            action: Some(format!(
                "Use the official module, or remove the third-party module from bucket {}.",
                hidden.bucket
            )),
        });
    }
    Ok(diagnostics)
}
