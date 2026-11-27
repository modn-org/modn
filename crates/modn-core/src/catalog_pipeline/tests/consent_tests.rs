//! Consent checks: stale plans and repeated capability coverage.

use super::synthetic::{approval, base_plan, consent_for, dependency_plan, pin, preview};
use crate::acquisition::ReleaseDigest;
use crate::approval::{ApprovalRefusal, Capabilities};
use crate::catalog_pipeline::{plan_digest, CatalogPlan, ConfirmedStack, StalePlan};
use crate::error::{code_of, ErrorCode};
use crate::transactions::{DependencyState, NativeInjectorNotAllowed, NativeInjectorPolicy};

#[test]
fn a_matching_consent_passes() {
    let plan = base_plan();
    let previews = [preview("top", 'a')];
    let consent = consent_for(&plan, false, Vec::new(), &previews);

    consent.check(&plan, &previews).expect("matching consent");
}

#[test]
fn a_changed_plan_returns_a_stale_refusal() {
    let reviewed = base_plan();
    let previews = [preview("top", 'a')];
    let consent = consent_for(&reviewed, false, Vec::new(), &previews);
    let mut changed = base_plan();
    changed.game_id = "other-game".to_owned();

    let error = consent
        .check(&changed, &previews)
        .expect_err("a changed plan is stale");

    assert_eq!(code_of(&error), Some(ErrorCode::PlanStale));
    let stale = error.downcast_ref::<StalePlan>().expect("typed refusal");
    assert_eq!(stale.expected, consent.plan);
    assert_eq!(stale.found, plan_digest(&changed, &[], &previews));
    assert_eq!(stale.input, Some("target"));
    assert_eq!(stale.code(), ErrorCode::PlanStale);
    assert!(stale.to_string().contains("run the command again"));
}

#[test]
fn a_native_injector_without_the_allowance_is_refused() {
    let mut plan = base_plan();
    plan.order[0].needed = Capabilities::new(true, std::iter::empty::<String>());
    plan.order[0].release.native_injector = true;
    assert!(plan.refusals.is_empty(), "the plan lists no refusal");
    let previews = [preview("top", 'a')];
    let consent = consent_for(&plan, false, Vec::new(), &previews);

    let error = consent
        .check(&plan, &previews)
        .expect_err("an uncovered native injector is refused");

    assert_eq!(code_of(&error), Some(ErrorCode::NativeInjectorNotAllowed));
    assert!(error.downcast_ref::<NativeInjectorNotAllowed>().is_some());
    assert!(error.to_string().contains("--allow-native-injector"));
}

#[test]
fn an_uncovered_executable_is_refused() {
    let mut plan = base_plan();
    plan.order[0].needed = Capabilities::new(false, ["tools/helper.exe"]);
    let previews = [preview("top", 'a')];
    let consent = consent_for(&plan, false, Vec::new(), &previews);

    let error = consent
        .check(&plan, &previews)
        .expect_err("an uncovered executable is refused");

    assert_eq!(code_of(&error), Some(ErrorCode::ApprovalRequired));
    let refusal = error
        .downcast_ref::<ApprovalRefusal>()
        .expect("typed refusal");
    assert_eq!(refusal.code(), ErrorCode::ApprovalRequired);
    assert_eq!(
        refusal.as_json()["uncovered"]["executables"][0],
        "tools/helper.exe"
    );
}

#[test]
fn approved_native_and_executable_capabilities_pass() {
    let mut plan = base_plan();
    plan.order[0].needed = Capabilities::new(true, ["tools/helper.exe"]);
    plan.order[0].release.native_injector = true;
    plan.native_injector = NativeInjectorPolicy::Allow;
    let approved = vec![approval("top", &["tools/helper.exe"])];
    let previews = [preview("top", 'a')];
    let consent = consent_for(&plan, true, approved, &previews);

    consent
        .check(&plan, &previews)
        .expect("approved capabilities");
}

#[test]
fn a_ready_dependency_needs_no_capabilities() {
    let mut plan = dependency_plan();
    plan.order[0].needed = Capabilities::new(true, ["tools/helper.exe"]);
    plan.order[0].state = Some(DependencyState::Ready {
        package_ids: Vec::new(),
    });
    let consent = consent_for(&plan, false, Vec::new(), &[]);

    consent
        .check(&plan, &[])
        .expect("a ready dependency is not placed again");
}

fn assert_stale(confirmed: &ConfirmedStack, plan: &CatalogPlan) {
    let error = confirmed
        .verify(plan)
        .expect_err("a changed stack is stale");
    assert_eq!(code_of(&error), Some(ErrorCode::PlanStale));
    assert!(
        error.to_string().contains("run the command again"),
        "{error}"
    );
}

#[test]
fn a_confirmed_stack_accepts_an_unchanged_plan() {
    let plan = dependency_plan();
    let confirmed = ConfirmedStack::of(&plan).expect("confirmed stack");

    confirmed.verify(&plan).expect("the same stack");
}

#[test]
fn a_dependency_installed_after_confirmation_makes_the_stack_stale() {
    let plan = dependency_plan();
    let confirmed = ConfirmedStack::of(&plan).expect("confirmed stack");
    let mut changed = dependency_plan();
    changed.order[0].state = Some(DependencyState::Ready {
        package_ids: Vec::new(),
    });

    assert_stale(&confirmed, &changed);
}

#[test]
fn a_reordered_stack_is_stale() {
    let plan = dependency_plan();
    let confirmed = ConfirmedStack::of(&plan).expect("confirmed stack");
    let mut changed = dependency_plan();
    changed.order.swap(0, 1);

    assert_stale(&confirmed, &changed);
}

#[test]
fn a_changed_release_version_makes_the_confirmed_stack_stale() {
    let plan = dependency_plan();
    let confirmed = ConfirmedStack::of(&plan).expect("confirmed stack");
    let mut changed = dependency_plan();
    changed.order[0].package.package.version = "2.0".to_owned();

    assert_stale(&confirmed, &changed);
}

#[test]
fn a_changed_release_digest_makes_the_confirmed_stack_stale() {
    let plan = dependency_plan();
    let confirmed = ConfirmedStack::of(&plan).expect("confirmed stack");
    let mut changed = dependency_plan();
    changed.order[0].digest = ReleaseDigest::pinned(&pin('c')).expect("digest");

    assert_stale(&confirmed, &changed);
}

#[test]
fn a_digest_learned_after_confirmation_does_not_make_the_stack_stale() {
    let mut plan = dependency_plan();
    plan.order[0].digest = ReleaseDigest::Unknown;
    let confirmed = ConfirmedStack::of(&plan).expect("confirmed stack");
    let mut changed = dependency_plan();
    changed.order[0].digest = ReleaseDigest::computed(&pin('c')).expect("digest");

    confirmed
        .verify(&changed)
        .expect("acquisition learned the digest");
}

#[test]
fn a_changed_native_injector_policy_makes_the_confirmed_stack_stale() {
    let plan = dependency_plan();
    let confirmed = ConfirmedStack::of(&plan).expect("confirmed stack");
    let mut changed = dependency_plan();
    changed.native_injector = NativeInjectorPolicy::Allow;

    assert_stale(&confirmed, &changed);
}

#[test]
fn changed_executable_coverage_makes_the_confirmed_stack_stale() {
    let plan = dependency_plan();
    let confirmed = ConfirmedStack::of(&plan).expect("confirmed stack");
    let mut changed = dependency_plan();
    changed.order[0].needed = Capabilities::new(false, ["tools/helper.exe"]);

    assert_stale(&confirmed, &changed);
}

#[test]
fn a_changed_stored_approval_makes_the_confirmed_stack_stale() {
    let plan = dependency_plan();
    let confirmed = ConfirmedStack::of(&plan).expect("confirmed stack");
    let mut changed = dependency_plan();
    changed.order[0].stored = Some(Capabilities::new(false, ["tools/helper.exe"]));

    assert_stale(&confirmed, &changed);
}
