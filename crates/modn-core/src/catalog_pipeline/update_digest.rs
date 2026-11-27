//! Canonical digest fields of one update plan.
//!
//! The fields name the installed package, the paths the update replaces, the
//! up-to-date flag and every dependent with its requirement. They join the
//! `target` input group of [`super::digest::plan_digest`].

use super::digest::GroupHasher;
use super::plan::{DependentRequirement, InstalledPackage, PlannedDependent, UpdatePlan};
use crate::buckets::PackageSource;

/// Encode the installed facts of one update plan.
pub(super) fn update_input(hasher: &mut GroupHasher, update: &UpdatePlan) {
    installed(hasher, &update.installed);
    match &update.installed_version {
        Some(version) => {
            hasher.flag(true);
            hasher.text(version);
        }
        None => hasher.flag(false),
    }
    let mut replaced: Vec<&str> = update.replaced.iter().map(String::as_str).collect();
    replaced.sort_unstable();
    hasher.count(replaced.len());
    for path in replaced {
        hasher.text(path);
    }
    hasher.flag(update.up_to_date);
    let mut dependents: Vec<&PlannedDependent> = update.dependents.iter().collect();
    dependents.sort_by(|left, right| left.package_id.cmp(&right.package_id));
    hasher.count(dependents.len());
    for dependent in dependents {
        dependent_input(hasher, dependent);
    }
}

fn installed(hasher: &mut GroupHasher, installed: &InstalledPackage) {
    hasher.text(&installed.id);
    hasher.flag(installed.pinned);
    hasher.text(&installed.archive_blake3);
    match &installed.source {
        Some(source) => {
            hasher.flag(true);
            source_input(hasher, source);
        }
        None => hasher.flag(false),
    }
}

fn dependent_input(hasher: &mut GroupHasher, dependent: &PlannedDependent) {
    hasher.text(&dependent.package_id);
    hasher.text(&dependent.package);
    hasher.flag(dependent.pinned);
    match &dependent.requirement {
        DependentRequirement::Unknown => hasher.text("unknown"),
        DependentRequirement::Unrestricted => hasher.text("unrestricted"),
        DependentRequirement::Requires(text) => {
            hasher.text("requires");
            hasher.text(text);
        }
        DependentRequirement::Alias(alias) => {
            hasher.text("alias");
            hasher.text(alias);
        }
    }
    hasher.flag(dependent.satisfied);
}

fn source_input(hasher: &mut GroupHasher, source: &PackageSource) {
    match source {
        PackageSource::Manual { filename } => {
            hasher.text("manual");
            hasher.text(filename);
        }
        PackageSource::Nexus {
            game_domain,
            mod_id,
            file_id,
            file_name,
        } => {
            hasher.text("nexus");
            hasher.text(game_domain);
            hasher.number(*mod_id);
            hasher.number(*file_id);
            hasher.text(file_name);
        }
        PackageSource::Https { url } => {
            hasher.text("https");
            hasher.text(url);
        }
        PackageSource::GithubRelease {
            repository,
            tag,
            asset,
        } => {
            hasher.text("github-release");
            hasher.text(repository);
            hasher.text(tag);
            hasher.text(asset);
        }
        PackageSource::Git { repository, commit } => {
            hasher.text("git");
            hasher.text(repository);
            hasher.text(commit);
        }
    }
}
