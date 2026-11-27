//! Plain-data consent for one catalog install plan.
//!
//! A [`Consent`] records what the user approved and the digest of the plan
//! they saw. It is not authorization by itself: a plan holds no consent, and
//! nothing here derives one from a plan. There is no `From<&CatalogPlan>`, no
//! `From<CatalogPlan>` and no constructor that takes a plan; the caller passes
//! every grant and the reviewed digest explicitly. The commit routine checks
//! the consent and repeats the capability checks on the plan's `needed`
//! capabilities.

use super::digest::{plan_digest, PlanDigest, ReleasePreview};
use super::plan::{CatalogPlan, PlannedRelease};
use crate::approval::{ApprovalRefusal, Capabilities, Uncovered};
use crate::error::{coded, ErrorCode};
use crate::transactions::{
    CatalogReference, DependencyState, NativeInjectorNotAllowed, NativeInjectorPolicy,
};
use anyhow::Result;
use std::collections::BTreeSet;
use std::fmt;

/// Approved executable payload paths of one planned release.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExecutableApproval {
    /// The release whose paths the user approved.
    pub reference: CatalogReference,
    /// Game-relative executable paths the user approved.
    pub executables: BTreeSet<String>,
}

impl ExecutableApproval {
    /// Record the paths the user approved for `reference`.
    #[must_use]
    pub fn new(reference: CatalogReference, executables: impl IntoIterator<Item = String>) -> Self {
        Self {
            reference,
            executables: executables.into_iter().collect(),
        }
    }
}

/// What one user approved for one plan, and the plan digest they saw.
///
/// A consent is plain data. It cannot be derived from a plan: this module has
/// no `From<&CatalogPlan>`, no `From<CatalogPlan>` and no constructor that
/// accepts a plan, and no default grants a capability. The caller names the
/// native-injector allowance, the approved executable paths per release and
/// the reviewed [`PlanDigest`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Consent {
    /// The user allowed native injectors in this plan.
    pub native_injector: bool,
    /// Approved executable paths, grouped by release.
    pub executables: Vec<ExecutableApproval>,
    /// The digest of the plan the user reviewed.
    pub plan: PlanDigest,
}

impl Consent {
    /// Record explicit grants and the digest the user saw.
    ///
    /// Every grant comes from the caller. The constructor reads no plan and
    /// derives nothing from one.
    #[must_use]
    pub fn new(
        native_injector: bool,
        executables: Vec<ExecutableApproval>,
        plan: PlanDigest,
    ) -> Self {
        Self {
            native_injector,
            executables,
            plan,
        }
    }

    /// Check this consent against `plan` and its policy `previews`.
    ///
    /// The check recomputes the plan digest and refuses a mismatch with a
    /// [`StalePlan`]. It then repeats the native-injector and executable
    /// coverage checks on every release the plan would install. It trusts
    /// neither the plan's native-injector policy nor its refusals.
    ///
    /// # Errors
    ///
    /// Returns a [`StalePlan`] for a different plan, a
    /// [`NativeInjectorNotAllowed`] for an uncovered native injector, or an
    /// [`ApprovalRefusal`] for uncovered executable paths.
    pub fn check(&self, plan: &CatalogPlan, previews: &[ReleasePreview]) -> Result<()> {
        let found = plan_digest(plan, &self.executables, previews);
        if found != self.plan {
            return Err(StalePlan::new(self.plan.clone(), found).into_error());
        }
        for planned in plan.members()? {
            let reference = planned.package.reference();
            let approved = self.approval_for(&reference);
            let uncovered = planned.needed.uncovered_by(&approved);
            if uncovered.native_injector {
                return Err(NativeInjectorNotAllowed::new(&reference).into());
            }
            if !uncovered.executables.is_empty() {
                return Err(ApprovalRefusal::new(
                    reference,
                    Uncovered {
                        native_injector: false,
                        executables: uncovered.executables,
                    },
                )
                .into_error());
            }
        }
        Ok(())
    }

    /// The capabilities this consent grants for one release.
    fn approval_for(&self, reference: &CatalogReference) -> Capabilities {
        let executables: Vec<String> = self
            .executables
            .iter()
            .filter(|approval| approval.reference == *reference)
            .flat_map(|approval| approval.executables.iter().cloned())
            .collect();
        Capabilities::new(self.native_injector, executables)
    }
}

/// The stack facts that one confirmation covers.
///
/// A stack prompt names the member references, versions and install order, the
/// plan's native-injector policy and the executable coverage evidence of each
/// member. The caller records them before the archive acquisition and checks
/// them again after the re-plan; a difference is a stale plan and writes
/// nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConfirmedStack {
    /// The plan-wide native-injector policy the user confirmed.
    pub native_injector: NativeInjectorPolicy,
    /// One entry per member, in install order.
    pub members: Vec<ConfirmedMember>,
}

/// The facts of one confirmed stack member.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConfirmedMember {
    /// The catalog identity that the prompt named.
    pub reference: CatalogReference,
    /// The release version that the prompt named.
    pub version: String,
    /// The release archive digest the plan pins, when one is known.
    ///
    /// Acquisition may learn the digest of a Git release after the prompt, so
    /// a missing confirmed digest is not compared.
    pub digest: Option<String>,
    /// The capabilities that the member needs.
    pub needed: Capabilities,
    /// The stored approval that covered part of `needed` at confirmation time.
    pub stored: Option<Capabilities>,
}

impl ConfirmedStack {
    /// Record the stack facts of `plan`.
    ///
    /// The result is plain data. A caller that calls this again after a
    /// re-plan gets an equal value only when the stack did not change.
    ///
    /// # Errors
    ///
    /// Returns the refusal of a shared archive that cannot be merged.
    pub fn of(plan: &CatalogPlan) -> Result<Self> {
        let members = plan.members()?;
        Ok(Self {
            native_injector: plan.native_injector,
            members: members
                .iter()
                .map(|planned| ConfirmedMember::of(planned))
                .collect(),
        })
    }

    /// Refuse `plan` when its stack facts differ from this confirmation.
    ///
    /// # Errors
    ///
    /// Returns a `PLAN_STALE` refusal that names the first difference and
    /// tells the user to run the command again.
    pub fn verify(&self, plan: &CatalogPlan) -> Result<()> {
        let found = Self::of(plan)?;
        let Some(difference) = self.difference(&found) else {
            return Ok(());
        };
        Err(coded(
            ErrorCode::PlanStale,
            format!(
                "the plan changed after you confirmed it; run the command again ({difference})"
            ),
        ))
    }

    /// The first difference between this confirmation and a fresh plan.
    fn difference(&self, found: &Self) -> Option<String> {
        if self.native_injector != found.native_injector {
            return Some("the native-injector policy changed".to_owned());
        }
        if self.members.len() != found.members.len() {
            return Some(format!(
                "the confirmed stack has {} member(s); the fresh plan has {}",
                self.members.len(),
                found.members.len()
            ));
        }
        for (index, (expected, actual)) in self.members.iter().zip(&found.members).enumerate() {
            if expected.reference != actual.reference {
                return Some(format!(
                    "member {} is {} in the fresh plan, not {}",
                    index + 1,
                    actual.reference,
                    expected.reference
                ));
            }
            let reference = &expected.reference;
            if expected.version != actual.version {
                return Some(format!(
                    "{reference}: the release version changed from {} to {}",
                    expected.version, actual.version
                ));
            }
            if expected
                .digest
                .as_ref()
                .is_some_and(|digest| actual.digest.as_deref() != Some(digest.as_str()))
            {
                return Some(format!("{reference}: the release archive changed"));
            }
            if expected.needed != actual.needed {
                return Some(format!("{reference}: the executable coverage changed"));
            }
            if expected.stored != actual.stored {
                return Some(format!("{reference}: the stored approval changed"));
            }
        }
        None
    }
}

impl ConfirmedMember {
    fn of(planned: &PlannedRelease) -> Self {
        Self {
            reference: planned.package.reference(),
            version: planned.package.package.version.clone(),
            digest: planned.digest.as_str().map(str::to_owned),
            needed: planned.needed.clone(),
            stored: planned.stored.clone(),
        }
    }
}

/// Refusal for a consent whose plan digest no longer matches the current plan.
///
/// The digest names a canonical input group that differs when both digests
/// record their groups. A mismatch never writes.
#[derive(Debug)]
#[non_exhaustive]
pub struct StalePlan {
    /// The digest the user reviewed.
    pub expected: PlanDigest,
    /// The digest of the plan at commit time.
    pub found: PlanDigest,
    /// The canonical input group that differs, when it is known.
    pub input: Option<&'static str>,
}

impl StalePlan {
    fn new(expected: PlanDigest, found: PlanDigest) -> Self {
        let input = expected.first_difference(&found);
        Self {
            expected,
            found,
            input,
        }
    }

    /// The stable code of this refusal.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        ErrorCode::PlanStale
    }

    /// Convert into an error that keeps both the stable code and this value.
    #[must_use]
    pub fn into_error(self) -> anyhow::Error {
        anyhow::Error::new(self)
    }
}

impl fmt::Display for StalePlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "the plan changed since it was made; run the command again (expected {}, found {})",
            self.expected, self.found
        )?;
        if let Some(input) = self.input {
            write!(formatter, "; the {input} input changed")?;
        }
        Ok(())
    }
}

impl std::error::Error for StalePlan {}

/// Whether this release would be placed by the plan.
///
/// The planner installs the selected release and every missing dependency.
/// Ready, damaged and other-release dependencies are not placed.
pub(crate) fn installs(planned: &PlannedRelease) -> bool {
    matches!(&planned.state, None | Some(DependencyState::Missing))
}
