#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "ownership/model.rs"]
mod model;

use model::{case, ownership_case, Case, Observed};
use proptest::prelude::*;
use proptest::test_runner::{Config, RngSeed, TestCaseError, TestError, TestRunner};

const FIXED_SEED: u64 = 0x6f77_6e65;

fn config(default_cases: u32) -> Config {
    let mut config = Config::default();
    if std::env::var_os("PROPTEST_CASES").is_none() {
        config.cases = default_cases;
    }
    if std::env::var_os("PROPTEST_RNG_SEED").is_none() {
        config.rng_seed = RngSeed::Fixed(FIXED_SEED);
    }
    config.failure_persistence = None;
    config.max_shrink_iters = 64;
    config
}

fn run_cases(
    default_cases: u32,
    check: impl Fn(&Observed) -> Result<(), TestCaseError>,
) -> (usize, usize) {
    let installs = std::cell::Cell::new(0usize);
    let switches = std::cell::Cell::new(0usize);
    let mut runner = TestRunner::new(config(default_cases));
    let result = runner.run(&case(), |case| {
        let mut observed = Observed::default();
        ownership_case(&case, &mut observed)?;
        installs.set(installs.get() + observed.installs);
        switches.set(switches.get() + observed.switches);
        check(&observed)
    });
    match result {
        Ok(()) => (installs.get(), switches.get()),
        Err(TestError::Fail(reason, case)) => {
            panic!("{reason}\nminimal counterexample: {case:#?}")
        }
        Err(TestError::Abort(reason)) => panic!("aborted: {reason}"),
    }
}

#[test]
fn dry_run_install_and_uninstall_keep_ownership_promises() {
    let (installs, switches) = run_cases(64, |_| Ok(()));
    println!(
        "ownership: {installs} approved installs, {switches} profile switch round trips checked"
    );
    assert!(installs >= 5, "too few approved installs: {installs}");
    assert!(
        switches >= 3,
        "too few profile switch round trips: {switches}"
    );
}

#[test]
fn single_component_destination_plans_installs_and_uninstalls() {
    let case = Case {
        user_files: Vec::new(),
        first: vec!["single.lua".into(), "Mods/inner.lua".into()],
        second: Vec::new(),
    };
    let mut observed = Observed::default();
    let result = ownership_case(&case, &mut observed);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(observed.installs, 1);
}

#[test]
fn reviewable_dry_run_predicts_the_install() {
    let (installs, _) = run_cases(256, |observed| {
        prop_assert!(
            observed.reviewable_install_failed.is_empty(),
            "reviewable dry run, failed install: {:?}",
            observed.reviewable_install_failed
        );
        Ok(())
    });
    println!("preview agreement: {installs} approved installs checked");
    assert!(installs >= 5, "too few approved installs: {installs}");
}
