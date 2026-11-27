//! The result of one committed catalog plan.

use super::plan::{CatalogPlan, PlanRefusal};
use crate::acquisition::SourceAttempt;
use crate::buckets::PackageSource;
use crate::error::{coded, ErrorCode};
use crate::transactions::ManagedPackage;
use std::collections::BTreeMap;

/// One committed catalog install, single or stack.
#[derive(Debug)]
#[non_exhaustive]
pub struct CommitOutcome {
    /// The managed packages the transaction installed, in install order.
    pub packages: Vec<ManagedPackage>,
    /// The operation history ID that the commit recorded, when it recorded one.
    pub operation_id: Option<String>,
    /// Every archive acquisition attempt, in acquisition order.
    pub attempts: Vec<SourceAttempt>,
    /// The source that served each package's archive, keyed by its
    /// `bucket/game/mod` reference.
    pub sources: BTreeMap<String, PackageSource>,
}

/// The first refusal that a consent cannot repair, as a coded error.
///
/// Capability refusals are not blocking: the consent repeats the
/// native-injector and executable checks itself. Every other refusal means the
/// plan cannot be committed as it stands.
pub(crate) fn blocking_refusal(plan: &CatalogPlan) -> Option<anyhow::Error> {
    plan.refusals.iter().find_map(|refusal| match refusal {
        PlanRefusal::NativeInjector { .. } | PlanRefusal::ApprovalRequired { .. } => None,
        PlanRefusal::Conflict { conflict } => Some(coded(
            ErrorCode::Conflict,
            format!(
                "{} conflicts with installed packages; uninstall them first: {}",
                conflict.reference,
                conflict.installed.join(", ")
            ),
        )),
        PlanRefusal::DependencyNotReady { reference, state } => Some(coded(
            ErrorCode::DependencyMissing,
            format!("dependency {reference} is not ready ({state:?}); install or restore it first"),
        )),
        PlanRefusal::ArchiveRequired { reference, .. } => Some(coded(
            ErrorCode::ArchiveRequired,
            format!("{reference}: no archive is available and no source can be downloaded"),
        )),
        PlanRefusal::ArchiveUnmatched { path } => Some(coded(
            ErrorCode::InvalidArguments,
            format!(
                "supplied archive matches no release in this plan: {}",
                path.display()
            ),
        )),
        PlanRefusal::SourceChanged { reference } => Some(coded(
            ErrorCode::DigestMismatch,
            format!("{reference}: source changed after catalog verification"),
        )),
        PlanRefusal::PreviewBlocked {
            reference,
            blockers,
        } => Some(coded(
            ErrorCode::PlanBlocked,
            format!(
                "{reference}: installation blocked: {}",
                blockers
                    .iter()
                    .map(|issue| format!("{}: {}", issue.code, issue.message))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        )),
    })
}
