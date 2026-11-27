//! Capabilities, stored approvals and the native-injector policy of one plan.
//!
//! The step reads stored approvals from the supplied store and folds them with
//! the command-line flags as data. It never prompts, grants nothing and writes
//! nothing. Uncovered capabilities become refusals for the caller to report.

use super::order::OrderedRelease;
use super::plan::PlanRefusal;
use crate::approval::{ApprovalRefusal, Capabilities, Uncovered};
use crate::state::Store;
use crate::transactions::NativeInjectorPolicy;
use anyhow::Result;
use std::path::Path;

/// The capability evidence of every release in an install order.
#[derive(Debug)]
pub(super) struct ResolvedRequirements {
    /// Capabilities each release declares, aligned with the order.
    pub(super) needed: Vec<Capabilities>,
    /// Stored approval of each release, aligned with the order.
    pub(super) stored: Vec<Option<Capabilities>>,
    /// Allow only when every installing native injector is covered.
    pub(super) native_injector: NativeInjectorPolicy,
    /// Installing native injectors that no flag or stored approval covers.
    pub(super) native_refusals: Vec<PlanRefusal>,
    /// Installing executables that no flag or stored approval covers.
    pub(super) approval_refusals: Vec<PlanRefusal>,
}

/// Read the capabilities and stored approvals of `order`, then decide the
/// native-injector policy and collect the refusals of the installing releases.
pub(super) fn resolve(
    order: &[OrderedRelease],
    installing: &[usize],
    store: Option<&Store>,
    installation: &Path,
    flags: &Capabilities,
) -> Result<ResolvedRequirements> {
    let mut needed = Vec::with_capacity(order.len());
    let mut stored = Vec::with_capacity(order.len());
    let mut uncovered = Vec::with_capacity(order.len());
    for ordered in order {
        let reference = ordered.package.reference();
        let capabilities = Capabilities::new(
            ordered.release.native_injector,
            ordered.package.package.executables.iter().cloned(),
        );
        let approval = match store {
            Some(store) => store.approval(installation, &reference.bucket, &reference.mod_id)?,
            None => None,
        };
        let approved = approval
            .as_ref()
            .map_or_else(|| flags.clone(), |stored| stored.union(flags));
        uncovered.push(capabilities.uncovered_by(&approved));
        needed.push(capabilities);
        stored.push(approval);
    }
    let mut native_refusals = Vec::new();
    let mut approval_refusals = Vec::new();
    for &index in installing {
        let reference = order[index].package.reference();
        let gaps = &uncovered[index];
        if needed[index].native_injector && gaps.native_injector {
            native_refusals.push(PlanRefusal::NativeInjector {
                reference: reference.clone(),
            });
        }
        if !gaps.executables.is_empty() {
            approval_refusals.push(PlanRefusal::ApprovalRequired {
                refusal: ApprovalRefusal::new(
                    reference,
                    Uncovered {
                        native_injector: false,
                        executables: gaps.executables.clone(),
                    },
                ),
            });
        }
    }
    let places_native = installing
        .iter()
        .any(|&index| needed[index].native_injector);
    let every_native_covered = installing
        .iter()
        .all(|&index| !uncovered[index].native_injector);
    let native_injector = if places_native && every_native_covered {
        NativeInjectorPolicy::Allow
    } else {
        NativeInjectorPolicy::Refuse
    };
    Ok(ResolvedRequirements {
        needed,
        stored,
        native_injector,
        native_refusals,
        approval_refusals,
    })
}
