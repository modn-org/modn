//! `modn catalog checkver`: stateless upstream checks for catalog packages.

use super::checkver_output as output;
use super::checkver_update;
use super::checkver_upstream;
use super::keys::require_directory;
use anyhow::{bail, Context, Result};
use modn_core::buckets::{Checkver, PackageManifest};
use modn_core::error::{coded, ErrorCode};
use std::fs;
use std::path::{Path, PathBuf};

const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Status {
    UpToDate,
    UpdateAvailable,
    Skipped,
    Error,
}

impl Status {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::UpToDate => "up_to_date",
            Self::UpdateAvailable => "update_available",
            Self::Skipped => "skipped",
            Self::Error => "error",
        }
    }
}

struct Package {
    game: String,
    id: String,
    path: PathBuf,
    manifest: Option<PackageManifest>,
    load_error: Option<String>,
}

pub(super) struct Report {
    pub(super) game: String,
    pub(super) id: String,
    pub(super) status: Status,
    pub(super) kind: Option<&'static str>,
    pub(super) current_version: Option<String>,
    pub(super) latest_version: Option<String>,
    pub(super) error: Option<String>,
    pub(super) applied: Option<UpdateResult>,
}

#[derive(Default)]
pub(super) struct Summary {
    pub(super) up_to_date: usize,
    pub(super) update_available: usize,
    pub(super) skipped: usize,
    pub(super) error: usize,
    pub(super) updated: usize,
}

/// The upstream values one update writes into a package manifest.
pub(super) enum Upstream {
    GithubRelease {
        repository: String,
        tag: String,
        asset: String,
        version: String,
    },
    Git {
        commit: String,
        version: String,
    },
}

/// What one successful manifest update did.
pub(super) struct UpdateResult {
    pub(super) old_version: String,
    pub(super) new_version: String,
    pub(super) marker: Marker,
    pub(super) kept_previous: Option<String>,
    pub(super) removed_upstream_sha256: bool,
    pub(super) other_sources: usize,
}

/// The stable marker of a rewritten manifest pin.
pub(super) enum Marker {
    Blake3(String),
    Commit(String),
}

#[derive(Clone, Copy)]
enum Mode {
    Check,
    Update { keep_previous: bool },
}

struct Checked {
    report: Report,
    upstream: Option<Upstream>,
}

pub(super) fn run(
    dir: &Path,
    target: Option<&str>,
    json_output: bool,
    update: bool,
    keep_previous: bool,
) -> Result<()> {
    require_directory(dir, "catalog directory")?;
    let mode = if update {
        Mode::Update { keep_previous }
    } else {
        Mode::Check
    };
    let packages = select(discover(dir)?, target)?;
    let mut reports = Vec::with_capacity(packages.len());
    for package in &packages {
        let mut checked = check(package);
        if let Mode::Update { keep_previous } = mode {
            if checked.report.status == Status::UpdateAvailable {
                apply(package, &mut checked, keep_previous);
            }
        }
        reports.push(checked.report);
    }
    let summary = summarize(&reports);
    if json_output {
        output::print_json(&reports, &summary, update);
    } else {
        output::print_human(&reports, &summary, update);
    }
    if summary.error > 0 {
        let what = if update {
            "package check or update step"
        } else {
            "package check"
        };
        return Err(coded(
            ErrorCode::BucketFailed,
            format!("{} {what}(s) failed", summary.error),
        ));
    }
    Ok(())
}

fn apply(package: &Package, checked: &mut Checked, keep_previous: bool) {
    let Some(manifest) = &package.manifest else {
        return;
    };
    let Some(upstream) = &checked.upstream else {
        if checked.report.kind == Some("nexus") {
            checked.report.status = Status::Error;
            checked.report.error = Some(output::NEXUS_UPDATE_UNSUPPORTED.to_owned());
        }
        return;
    };
    match checkver_update::apply(
        &package.game,
        &package.id,
        &package.path,
        manifest,
        upstream,
        keep_previous,
    ) {
        Ok(result) => checked.report.applied = Some(result),
        Err(error) => {
            checked.report.status = Status::Error;
            checked.report.error = Some(format!("{error:#}"));
        }
    }
}

fn discover(dir: &Path) -> Result<Vec<Package>> {
    let mods = dir.join("mods");
    if fs::symlink_metadata(&mods).is_err() {
        return Ok(Vec::new());
    }
    let mut packages = Vec::new();
    let mut games: Vec<_> = fs::read_dir(&mods)
        .with_context(|| format!("cannot read catalog packages: {}", mods.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    games.sort_by_key(std::fs::DirEntry::file_name);
    for game in games {
        if !game.file_type()?.is_dir() {
            continue;
        }
        let game_id = game.file_name().to_string_lossy().into_owned();
        let mut records: Vec<_> = fs::read_dir(game.path())
            .with_context(|| {
                format!(
                    "cannot read catalog game directory: {}",
                    game.path().display()
                )
            })?
            .collect::<std::io::Result<Vec<_>>>()?;
        records.sort_by_key(std::fs::DirEntry::file_name);
        for entry in records {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(std::ffi::OsStr::to_str) else {
                continue;
            };
            let Some(id) = name.strip_suffix(".json") else {
                continue;
            };
            if id.is_empty() || id.contains('@') || !entry.file_type()?.is_file() {
                continue;
            }
            packages.push(read_package(&game_id, id, &path));
        }
    }
    packages.sort_by(|left, right| (&left.game, &left.id).cmp(&(&right.game, &right.id)));
    Ok(packages)
}

fn read_package(game: &str, id: &str, path: &Path) -> Package {
    let loaded =
        read_manifest(path).and_then(|manifest| manifest.validate(game, id).map(|()| manifest));
    match loaded {
        Ok(manifest) => Package {
            game: game.to_owned(),
            id: id.to_owned(),
            path: path.to_owned(),
            manifest: Some(manifest),
            load_error: None,
        },
        Err(error) => Package {
            game: game.to_owned(),
            id: id.to_owned(),
            path: path.to_owned(),
            manifest: None,
            load_error: Some(format!("{error:#}")),
        },
    }
}

fn read_manifest(path: &Path) -> Result<PackageManifest> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("cannot read package manifest: {}", path.display()))?;
    if !metadata.is_file() {
        bail!(
            "package manifest must be a regular file: {}",
            path.display()
        );
    }
    if metadata.len() > MAX_MANIFEST_BYTES {
        bail!("package manifest exceeds 1 MiB: {}", path.display());
    }
    let bytes = fs::read(path)
        .with_context(|| format!("cannot read package manifest: {}", path.display()))?;
    serde_json::from_slice(&bytes).context("invalid package manifest")
}

fn select(packages: Vec<Package>, target: Option<&str>) -> Result<Vec<Package>> {
    let Some(target) = target else {
        return Ok(packages);
    };
    let (game, id) = match target.split_once('/') {
        Some((game, id)) if !game.is_empty() && !id.is_empty() && !id.contains('/') => {
            (Some(game), id)
        }
        Some(_) => bail!("invalid package reference; expected <mod-id> or <game-id>/<mod-id>"),
        None => (None, target),
    };
    if id.is_empty() {
        bail!("invalid package reference; expected <mod-id> or <game-id>/<mod-id>");
    }
    let matches: Vec<Package> = packages
        .into_iter()
        .filter(|package| package.id == id && game.is_none_or(|game| package.game == game))
        .collect();
    match matches.len() {
        0 => bail!("catalog package not found: {target}"),
        1 => Ok(matches),
        _ => {
            let games: Vec<&str> = matches
                .iter()
                .map(|package| package.game.as_str())
                .collect();
            bail!(
                "catalog package {target} is ambiguous; it exists in games {}; use <game-id>/<mod-id>",
                games.join(", ")
            )
        }
    }
}

fn check(package: &Package) -> Checked {
    let Some(manifest) = &package.manifest else {
        return Checked {
            report: Report {
                game: package.game.clone(),
                id: package.id.clone(),
                status: Status::Error,
                kind: None,
                current_version: None,
                latest_version: None,
                error: package.load_error.clone(),
                applied: None,
            },
            upstream: None,
        };
    };
    let Some(checkver) = &manifest.checkver else {
        return Checked {
            report: Report {
                game: package.game.clone(),
                id: package.id.clone(),
                status: Status::Skipped,
                kind: None,
                current_version: Some(manifest.version.clone()),
                latest_version: None,
                error: None,
                applied: None,
            },
            upstream: None,
        };
    };
    let kind = Some(kind_of(checkver));
    match checkver_upstream::check_upstream(manifest, checkver) {
        Ok((latest, update, upstream)) => Checked {
            report: Report {
                game: package.game.clone(),
                id: package.id.clone(),
                status: if update {
                    Status::UpdateAvailable
                } else {
                    Status::UpToDate
                },
                kind,
                current_version: Some(manifest.version.clone()),
                latest_version: latest,
                error: None,
                applied: None,
            },
            upstream,
        },
        Err(error) => Checked {
            report: Report {
                game: package.game.clone(),
                id: package.id.clone(),
                status: Status::Error,
                kind,
                current_version: Some(manifest.version.clone()),
                latest_version: None,
                error: Some(format!("{error:#}")),
                applied: None,
            },
            upstream: None,
        },
    }
}

fn kind_of(checkver: &Checkver) -> &'static str {
    match checkver {
        Checkver::GithubRelease { .. } => "github-release",
        Checkver::Git { .. } => "git",
        Checkver::Nexus { .. } => "nexus",
        _ => "unknown",
    }
}

fn summarize(reports: &[Report]) -> Summary {
    let mut summary = Summary::default();
    for report in reports {
        if report.applied.is_some() {
            summary.updated += 1;
        }
        match report.status {
            Status::UpToDate => summary.up_to_date += 1,
            Status::UpdateAvailable => summary.update_available += 1,
            Status::Skipped => summary.skipped += 1,
            Status::Error => summary.error += 1,
        }
    }
    summary
}
