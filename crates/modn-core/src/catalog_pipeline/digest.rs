//! Canonical fingerprints of one catalog install plan.
//!
//! A [`PlanDigest`] covers the facts that decide what an installation writes
//! and what the user approved. It excludes the acquisition state (archive
//! paths, served status and pending downloads), because a commit may acquire
//! those after the user consented. The encoding is length-prefixed, sorted by
//! canonical keys and independent of map or set iteration order. BLAKE3 is the
//! only hash.

use super::consent::ExecutableApproval;
use super::plan::{CatalogPlan, PlanConflict, PlanKind};
use crate::approval::Capabilities;
use crate::planning::{InstallPlan, PlanIssue, PlanStatus, PlannedFile};
use crate::transactions::{
    CatalogConflict, CatalogDependency, CatalogReference, DependencyState, NativeInjectorPolicy,
};
use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

/// A canonical BLAKE3 fingerprint of one plan's identity facts.
///
/// Two plans that differ in what they would write, or in what the user
/// approved, produce different digests on every platform. The digest orders
/// every collection by a canonical key and never depends on iteration order.
#[derive(Clone, PartialEq, Eq)]
pub struct PlanDigest {
    hex: String,
    inputs: Vec<(&'static str, String)>,
}

impl PlanDigest {
    /// The digest as 64 lowercase hexadecimal characters.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.hex
    }

    /// The canonical input group that differs, when both digests list it.
    pub(crate) fn first_difference(&self, other: &Self) -> Option<&'static str> {
        for ((name, left), (_, right)) in self.inputs.iter().zip(&other.inputs) {
            if left != right {
                return Some(*name);
            }
        }
        None
    }
}

impl fmt::Debug for PlanDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("PlanDigest")
            .field(&self.hex)
            .finish()
    }
}

impl fmt::Display for PlanDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.hex)
    }
}

/// The game module's policy preview of one planned release.
///
/// The commit routine produces previews after archive acquisition. A plan
/// holds none, because a preview runs a game module and reads the archive.
#[derive(Debug)]
#[non_exhaustive]
pub struct ReleasePreview {
    /// The release the preview describes.
    pub reference: CatalogReference,
    /// The metadata-only installation plan of that release's archive.
    pub plan: InstallPlan,
}

impl ReleasePreview {
    /// Whether the game module allows this release's installation as previewed.
    #[must_use]
    pub fn reviewable(&self) -> bool {
        self.plan.status == PlanStatus::Reviewable && self.plan.blockers.is_empty()
    }
}

/// Fingerprint the identity facts of `plan`, `approved` and `previews`.
///
/// The digest includes the install order, each release's reference, version
/// and digest, dependency and conflict facts, the native-injector policy, the
/// approved executable paths, the per-release policy previews, the game id and
/// the canonical installation path. It excludes archive paths, served status
/// and pending downloads, because acquisition may change them after consent.
#[must_use]
pub fn plan_digest(
    plan: &CatalogPlan,
    approved: &[ExecutableApproval],
    previews: &[ReleasePreview],
) -> PlanDigest {
    let inputs = vec![
        ("target", target_input(plan)),
        ("order", order_input(plan)),
        ("dependencies", dependencies_input(plan)),
        ("conflicts", conflicts_input(plan)),
        ("native_injector", native_injector_input(plan)),
        ("executables", executables_input(approved)),
        ("preview", preview_input(previews)),
    ];
    let hex = combine(&inputs);
    PlanDigest { hex, inputs }
}

/// One named group of the canonical encoding.
struct GroupHasher {
    hasher: blake3::Hasher,
}

impl GroupHasher {
    fn new(name: &str) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"modn.plan.digest.group.v1");
        field(&mut hasher, name.as_bytes());
        Self { hasher }
    }

    fn text(&mut self, value: &str) {
        field(&mut self.hasher, value.as_bytes());
    }

    fn path(&mut self, path: &Path) {
        self.text(&normalized_path(path));
    }

    fn flag(&mut self, value: bool) {
        self.hasher.update(&[u8::from(value)]);
    }

    fn count(&mut self, value: usize) {
        self.number(u64::try_from(value).expect("collection length fits in u64"));
    }

    fn number(&mut self, value: u64) {
        self.hasher.update(&value.to_le_bytes());
    }

    fn finish(self) -> String {
        self.hasher.finalize().to_hex().to_string()
    }
}

/// Length-prefix one field so no two sequences encode the same bytes.
fn field(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(
        &u64::try_from(bytes.len())
            .expect("field length fits in u64")
            .to_le_bytes(),
    );
    hasher.update(bytes);
}

/// Combine the named group digests in their fixed order.
fn combine(inputs: &[(&'static str, String)]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"modn.plan.digest.v1");
    hasher.update(
        &u64::try_from(inputs.len())
            .expect("group count fits in u64")
            .to_le_bytes(),
    );
    for (name, digest) in inputs {
        field(&mut hasher, name.as_bytes());
        field(&mut hasher, digest.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn normalized_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn reference(hasher: &mut GroupHasher, reference: &CatalogReference) {
    hasher.text(&reference.bucket);
    hasher.text(&reference.game_id);
    hasher.text(&reference.mod_id);
}

fn reference_key(reference: &CatalogReference) -> (&str, &str, &str) {
    (&reference.bucket, &reference.game_id, &reference.mod_id)
}

fn target_input(plan: &CatalogPlan) -> String {
    let mut hasher = GroupHasher::new("target");
    hasher.text(match plan.kind {
        PlanKind::Install => "install",
    });
    reference(&mut hasher, &plan.selected);
    hasher.text(&plan.game_id);
    hasher.path(&plan.installation);
    hasher.finish()
}

fn order_input(plan: &CatalogPlan) -> String {
    let mut hasher = GroupHasher::new("order");
    hasher.count(plan.order.len());
    for planned in &plan.order {
        reference(&mut hasher, &planned.package.reference());
        hasher.text(&planned.package.package.version);
        match &planned.release.version {
            Some(version) => {
                hasher.flag(true);
                hasher.text(version);
            }
            None => hasher.flag(false),
        }
        match planned.digest.as_str() {
            Some(digest) => {
                hasher.flag(true);
                hasher.text(digest);
            }
            None => hasher.flag(false),
        }
        hasher.flag(planned.needed.native_injector);
        hasher.count(planned.needed.executables.len());
        for path in &planned.needed.executables {
            hasher.text(path);
        }
    }
    hasher.finish()
}

fn dependencies_input(plan: &CatalogPlan) -> String {
    let mut hasher = GroupHasher::new("dependencies");
    hasher.count(plan.order.len());
    for planned in &plan.order {
        reference(&mut hasher, &planned.package.reference());
        let mut declared: Vec<&CatalogDependency> = planned.release.dependencies.iter().collect();
        declared.sort_by(|left, right| {
            reference_key(&left.reference).cmp(&reference_key(&right.reference))
        });
        hasher.count(declared.len());
        for dependency in declared {
            reference(&mut hasher, &dependency.reference);
            match dependency.archive_blake3.as_str() {
                Some(digest) => {
                    hasher.flag(true);
                    hasher.text(digest);
                }
                None => hasher.flag(false),
            }
            match &dependency.requirement {
                Some(requirement) => {
                    hasher.flag(true);
                    hasher.text(requirement);
                }
                None => hasher.flag(false),
            }
        }
        dependency_state(&mut hasher, planned.state.as_ref());
    }
    hasher.finish()
}

fn dependency_state(hasher: &mut GroupHasher, state: Option<&DependencyState>) {
    match state {
        None => hasher.text("selected"),
        Some(DependencyState::Ready { package_ids }) => {
            hasher.text("ready");
            sorted_ids(hasher, package_ids);
        }
        Some(DependencyState::Damaged { package_ids }) => {
            hasher.text("damaged");
            sorted_ids(hasher, package_ids);
        }
        Some(DependencyState::OtherRelease { package_ids }) => {
            hasher.text("other_release");
            sorted_ids(hasher, package_ids);
        }
        Some(DependencyState::Missing) => hasher.text("missing"),
    }
}

fn sorted_ids(hasher: &mut GroupHasher, ids: &[String]) {
    let sorted: BTreeSet<&str> = ids.iter().map(String::as_str).collect();
    hasher.count(sorted.len());
    for id in sorted {
        hasher.text(id);
    }
}

fn conflicts_input(plan: &CatalogPlan) -> String {
    let mut hasher = GroupHasher::new("conflicts");
    let mut conflicts: Vec<&PlanConflict> = plan.conflicts.iter().collect();
    conflicts.sort_by(|left, right| {
        reference_key(&left.reference).cmp(&reference_key(&right.reference))
    });
    hasher.count(conflicts.len());
    for conflict in conflicts {
        reference(&mut hasher, &conflict.reference);
        sorted_ids(&mut hasher, &conflict.installed);
    }
    for planned in &plan.order {
        reference(&mut hasher, &planned.package.reference());
        let mut declared: Vec<&CatalogConflict> = planned.release.conflicts.iter().collect();
        declared.sort_by(|left, right| {
            reference_key(&left.reference).cmp(&reference_key(&right.reference))
        });
        hasher.count(declared.len());
        for conflict in declared {
            reference(&mut hasher, &conflict.reference);
            match &conflict.archive_blake3 {
                Some(digest) => {
                    hasher.flag(true);
                    hasher.text(digest);
                }
                None => hasher.flag(false),
            }
        }
    }
    hasher.finish()
}

fn native_injector_input(plan: &CatalogPlan) -> String {
    let mut hasher = GroupHasher::new("native_injector");
    hasher.text(match plan.native_injector {
        NativeInjectorPolicy::Allow => "allow",
        NativeInjectorPolicy::Refuse => "refuse",
    });
    hasher.finish()
}

fn executables_input(approved: &[ExecutableApproval]) -> String {
    let mut hasher = GroupHasher::new("executables");
    let mut approvals: Vec<&ExecutableApproval> = approved.iter().collect();
    approvals.sort_by(|left, right| {
        reference_key(&left.reference).cmp(&reference_key(&right.reference))
    });
    hasher.count(approvals.len());
    for approval in approvals {
        reference(&mut hasher, &approval.reference);
        let normalized = Capabilities::new(false, approval.executables.iter().cloned());
        hasher.count(normalized.executables.len());
        for path in &normalized.executables {
            hasher.text(path);
        }
    }
    hasher.finish()
}

fn preview_input(previews: &[ReleasePreview]) -> String {
    let mut hasher = GroupHasher::new("preview");
    let mut ordered: Vec<&ReleasePreview> = previews.iter().collect();
    ordered.sort_by(|left, right| {
        reference_key(&left.reference).cmp(&reference_key(&right.reference))
    });
    hasher.count(ordered.len());
    for preview in ordered {
        reference(&mut hasher, &preview.reference);
        preview_plan(&mut hasher, &preview.plan);
    }
    hasher.finish()
}

fn preview_plan(hasher: &mut GroupHasher, plan: &InstallPlan) {
    hasher.text(&plan.archive_blake3);
    hasher.path(&plan.game_installation);
    hasher.text(match plan.status {
        PlanStatus::Reviewable => "reviewable",
        PlanStatus::Blocked => "blocked",
    });
    hasher.flag(plan.metadata_only);
    let mut operations: Vec<&PlannedFile> = plan.operations.iter().collect();
    operations.sort_by(|left, right| {
        (
            left.root.as_str(),
            left.destination.as_str(),
            left.source.as_str(),
            left.kind.as_str(),
        )
            .cmp(&(
                right.root.as_str(),
                right.destination.as_str(),
                right.source.as_str(),
                right.kind.as_str(),
            ))
    });
    hasher.count(operations.len());
    for operation in operations {
        hasher.text(&operation.source);
        hasher.text(&operation.destination);
        hasher.number(operation.bytes);
        hasher.text(&operation.kind);
        match &operation.owned_directory {
            Some(directory) => {
                hasher.flag(true);
                hasher.text(directory);
            }
            None => hasher.flag(false),
        }
        hasher.text(&operation.root);
    }
    let mut blockers: Vec<&PlanIssue> = plan.blockers.iter().collect();
    blockers.sort_by(|left, right| {
        (left.code.as_str(), left.message.as_str())
            .cmp(&(right.code.as_str(), right.message.as_str()))
    });
    hasher.count(blockers.len());
    for blocker in blockers {
        hasher.text(&blocker.code);
        hasher.text(&blocker.message);
    }
    let mut skipped: Vec<&str> = plan
        .skipped_generated_files
        .iter()
        .map(String::as_str)
        .collect();
    skipped.sort_unstable();
    hasher.count(skipped.len());
    for path in skipped {
        hasher.text(path);
    }
    match &plan.content_base {
        Some(path) => {
            hasher.flag(true);
            hasher.path(path);
        }
        None => hasher.flag(false),
    }
    match &plan.content_root {
        Some(path) => {
            hasher.flag(true);
            hasher.path(path);
        }
        None => hasher.flag(false),
    }
}
