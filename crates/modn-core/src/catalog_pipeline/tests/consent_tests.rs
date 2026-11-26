//! Consent checks: stale plans and repeated capability coverage.

use super::synthetic::{approval, base_plan, consent_for, dependency_plan, preview};
use crate::approval::{ApprovalRefusal, Capabilities};
use crate::catalog_pipeline::{plan_digest, StalePlan};
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
