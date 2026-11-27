//! The plan input constructors a caller outside `modn-core` uses.

use super::fixtures::{self, package, Fixture, GAME};
use crate::acquisition::CacheUse;
use crate::approval::Capabilities;
use crate::catalog_pipeline::{plan_install, ConsentInputs, InstallInput, InstallTarget};

#[test]
fn input_constructors_build_one_plan_from_external_values() {
    let fixture = Fixture::new();
    let game = fixture.game();
    let archive = fixture.zip("top");
    fixture.add(&fixtures::manifest(
        "top",
        Some(&fixtures::digest(&archive)),
    ));
    let snapshot = fixture.snapshot();
    let selected = package(&snapshot, "top");
    let cache = fixture.cache();
    let target = InstallTarget::new(GAME, &game);
    assert_eq!(target.game_id, GAME);
    assert_eq!(target.installation, game);

    let consent = ConsentInputs::new(Capabilities::new(true, ["engine/tools/scc.exe"]));
    assert!(consent.flags.native_injector);

    let input = InstallInput::new(
        &selected,
        &snapshot,
        None,
        target,
        std::slice::from_ref(&archive),
        &cache,
        CacheUse::Reuse,
        consent,
    );
    let plan = plan_install(&input).expect("plan");
    assert_eq!(plan.selected.to_string(), fixtures::reference_of("top"));
    assert!(plan.order[0].served.is_some());
}
