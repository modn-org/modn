#![allow(
    dead_code,
    reason = "the shared profile fixture exposes helpers for other test crates"
)]

//! Milestone 20 end-to-end CLI gate.
//!
//! One journey installs overlapping mods, inspects the winner, raises a mod's
//! priority, switches profiles, recovers an interrupted switch and uninstalls
//! everything. Every command runs against a disposable game tree and manager
//! state; `LOCALAPPDATA`, `APPDATA` and `MODN_APPDATA` point at the fixture.

#[path = "support/profile_fixture.rs"]
mod profile_fixture;
#[path = "support/profiles_gate_fixture.rs"]
mod profiles_gate_fixture;

use profile_fixture::ProfileFixture;
use profiles_gate_fixture::{
    assert_one_active, create_second, install_alpha, install_beta, install_gamma, interrupt_switch,
    snapshot, uninstall, ALPHA_SHARED, ALPHA_UNIQUE, ALPHA_UNIQUE_BYTES, BETA_SHARED, BETA_UNIQUE,
    BETA_UNIQUE_BYTES, GAMMA, GAMMA_BYTES, SHARED, USER, USER_BYTES,
};
use serde_json::Value;
use std::fs;

fn bytes(fixture: &ProfileFixture, path: &str) -> Vec<u8> {
    fs::read(fixture.game().join(path)).expect("game file")
}

fn priority_winner(report: &Value) -> String {
    report["conflicts"][0]["claims"][0]["package"]
        .as_str()
        .expect("priority winner")
        .to_owned()
}

fn conflict(report: &Value) -> &Value {
    &report["conflicts"].as_array().expect("conflicts")[0]
}

#[test]
fn exit_gate_journey() {
    // Step 0: fresh installation with one unmanaged file.
    let fixture = ProfileFixture::new();
    fs::create_dir_all(fixture.game().join("archive/pc/mod")).expect("unmanaged folder");
    fs::write(fixture.game().join(USER), USER_BYTES).expect("unmanaged file");
    let initial = snapshot(fixture.game());
    assert_eq!(
        assert_one_active(&fixture, "step 0"),
        "default",
        "step 0: the fixture starts with one active profile"
    );

    // Step 1: install A then B, both claiming the shared path. A later install
    // wins, and both unique files plus the unmanaged file stay in place.
    let alpha = install_alpha(&fixture);
    assert_eq!(
        bytes(&fixture, SHARED),
        ALPHA_SHARED,
        "step 1: installing A writes A's shared bytes"
    );
    assert_eq!(
        bytes(&fixture, ALPHA_UNIQUE),
        ALPHA_UNIQUE_BYTES,
        "step 1: installing A writes its unique file"
    );
    let beta = install_beta(&fixture);
    assert_eq!(
        bytes(&fixture, SHARED),
        BETA_SHARED,
        "step 1: installing B last makes B rank 1 and writes B's shared bytes"
    );
    assert_eq!(
        bytes(&fixture, ALPHA_UNIQUE),
        ALPHA_UNIQUE_BYTES,
        "step 1: B leaves A's unique file untouched"
    );
    assert_eq!(
        bytes(&fixture, BETA_UNIQUE),
        BETA_UNIQUE_BYTES,
        "step 1: installing B writes its unique file"
    );
    assert_eq!(
        bytes(&fixture, USER),
        USER_BYTES,
        "step 1: installs leave the unmanaged file byte-identical"
    );
    assert_eq!(
        assert_one_active(&fixture, "step 1"),
        "default",
        "step 1: installing mods does not change the active profile"
    );

    // Step 2: conflicts sees exactly one overlapping path, won by B.
    let report = fixture.json(&["conflicts"]);
    let conflicts = report["conflicts"].as_array().expect("conflicts");
    assert_eq!(
        conflicts.len(),
        1,
        "step 2: exactly one overlapping path is reported"
    );
    let entry = conflict(&report);
    assert_eq!(
        entry["path"], SHARED,
        "step 2: the conflict names the shared path"
    );
    assert_eq!(entry["winner"], beta, "step 2: B is the on-disk winner");
    assert_eq!(
        entry["consistent"], true,
        "step 2: the on-disk winner matches the priority order"
    );
    let claims = entry["claims"].as_array().expect("claims");
    assert_eq!(claims.len(), 2, "step 2: both mods claim the path");
    assert_eq!(
        claims[0]["package"], beta,
        "step 2: claims are ordered B then A, so B is first"
    );
    assert_eq!(claims[1]["package"], alpha, "step 2: A is the second claim");
    assert_eq!(claims[0]["rank"], 1, "step 2: B has rank 1");
    assert_eq!(claims[1]["rank"], 2, "step 2: A has rank 2");
    let human = fixture.run(&["conflicts"]);
    assert!(
        human.status.success(),
        "step 2: the human report succeeds: {}",
        String::from_utf8_lossy(&human.stderr)
    );
    let text = String::from_utf8(human.stdout).expect("UTF-8 output");
    assert!(
        text.contains(&alpha) && text.contains(&beta),
        "step 2: the human report names both mods: {text}"
    );
    assert!(
        text.contains("1 overlapping files"),
        "step 2: the human report counts one overlap: {text}"
    );
    assert_eq!(
        assert_one_active(&fixture, "step 2"),
        "default",
        "step 2: conflicts does not change the active profile"
    );

    // Step 3: a priority change without --apply moves only the rank. Conflicts
    // then names A as the priority winner but reports the mismatch. Applying
    // makes A's bytes active and touches nothing else.
    let before_move = snapshot(fixture.game());
    let moved = fixture.json(&["profile", "priority", &alpha, "--rank", "1"]);
    assert_eq!(
        moved["applied"], false,
        "step 3: a priority change without --apply reports applied false"
    );
    assert_eq!(moved["rank"], 1, "step 3: A becomes rank 1");
    assert_eq!(
        snapshot(fixture.game()),
        before_move,
        "step 3: a rank change without --apply writes no game file"
    );
    let report = fixture.json(&["conflicts"]);
    let entry = conflict(&report);
    assert_eq!(
        priority_winner(&report),
        alpha,
        "step 3: conflicts marks A as the priority winner"
    );
    assert_eq!(
        entry["winner"], beta,
        "step 3: the on-disk winner is still B before apply"
    );
    assert_eq!(
        entry["consistent"], false,
        "step 3: conflicts reports the stored winner as inconsistent"
    );
    let before_apply = snapshot(fixture.game());
    let applied = fixture.json(&["profile", "apply", "--yes"]);
    assert_eq!(applied["dry_run"], false, "step 3: apply is a real run");
    let changes = applied["changes"].as_array().expect("changes");
    assert_eq!(changes.len(), 1, "step 3: apply changes only the overlap");
    assert_eq!(changes[0]["path"], SHARED, "step 3: the change is P");
    assert_eq!(
        changes[0]["action"], "replace",
        "step 3: apply replaces the shared file"
    );
    assert_eq!(changes[0]["from"], beta, "step 3: the replaced winner is B");
    assert_eq!(changes[0]["to"], alpha, "step 3: the new winner is A");
    assert_eq!(
        bytes(&fixture, SHARED),
        ALPHA_SHARED,
        "step 3: applying writes A's shared bytes"
    );
    let mut expected = before_apply;
    expected.insert(SHARED.into(), Some(ALPHA_SHARED.to_vec()));
    assert_eq!(
        snapshot(fixture.game()),
        expected,
        "step 3: apply touches only the overlapping path"
    );
    let report = fixture.json(&["conflicts"]);
    let entry = conflict(&report);
    assert_eq!(entry["winner"], alpha, "step 3: A is now on disk");
    assert_eq!(
        entry["consistent"], true,
        "step 3: the applied order is consistent"
    );
    assert_eq!(
        assert_one_active(&fixture, "step 3"),
        "default",
        "step 3: apply keeps the default profile active"
    );

    // Step 4: create a second profile, preview the switch, switch, install C,
    // and switch back to restore the step 3 tree byte for byte.
    create_second(&fixture);
    let before_dry_run = snapshot(fixture.game());
    let list_before = fixture.profile_list();
    let dry_run = fixture.json(&["profile", "switch", "second", "--dry-run"]);
    assert_eq!(dry_run["dry_run"], true, "step 4: the switch is a dry run");
    assert_eq!(
        dry_run["from"], "default",
        "step 4: the switch starts at default"
    );
    assert_eq!(dry_run["to"], "second", "step 4: the switch targets second");
    assert_eq!(
        dry_run["remove"].as_array().expect("remove").len(),
        3,
        "step 4: the preview lists the three default files"
    );
    assert_eq!(
        snapshot(fixture.game()),
        before_dry_run,
        "step 4: the switch dry run changes no byte"
    );
    assert_eq!(
        fixture.profile_list(),
        list_before,
        "step 4: the switch dry run changes no profile state"
    );
    let switched = fixture.json(&["profile", "switch", "second", "--yes"]);
    assert_eq!(
        switched["to"], "second",
        "step 4: the switch reports its target"
    );
    assert_eq!(
        assert_one_active(&fixture, "step 4 switch"),
        "second",
        "step 4: second becomes active"
    );
    assert!(
        !fixture.game().join(SHARED).exists()
            && !fixture.game().join(ALPHA_UNIQUE).exists()
            && !fixture.game().join(BETA_UNIQUE).exists(),
        "step 4: the default profile's files are gone from disk"
    );
    assert_eq!(
        bytes(&fixture, USER),
        USER_BYTES,
        "step 4: the switch leaves the unmanaged file byte-identical"
    );
    let gamma = install_gamma(&fixture);
    assert_eq!(
        bytes(&fixture, GAMMA),
        GAMMA_BYTES,
        "step 4: mod C is present in the second profile"
    );
    assert_eq!(
        bytes(&fixture, USER),
        USER_BYTES,
        "step 4: installing C leaves the unmanaged file byte-identical"
    );
    fixture.json(&["profile", "switch", "default", "--yes"]);
    assert_eq!(
        snapshot(fixture.game()),
        before_dry_run,
        "step 4: switching back restores the step 3 tree byte for byte"
    );
    assert_eq!(
        assert_one_active(&fixture, "step 4 switch back"),
        "default",
        "step 4: default is active again"
    );

    // Step 5: simulate a crash during default -> second, refuse the next write,
    // recover to the old profile, then repeat the switch successfully.
    let step4_tree = snapshot(fixture.game());
    let witness = interrupt_switch(&fixture, "txn-gate-switch-crashed", &alpha, &beta);
    assert_ne!(
        snapshot(fixture.game()),
        step4_tree,
        "step 5: the interrupted switch leaves a different tree"
    );
    assert!(
        witness.exists(),
        "step 5: the quarantine witness exists before recovery"
    );
    let (code, message) = fixture.error(&["profile", "switch", "second", "--yes"]);
    assert_eq!(
        code, "PENDING_TRANSACTION",
        "step 5: the next write is refused while a transaction is pending: {message}"
    );
    assert!(
        message.contains("modn recover"),
        "step 5: the refusal names the recovery command: {message}"
    );
    let recovered = fixture.json(&["recover", "--yes"]);
    assert_eq!(
        recovered["recovered"], 1,
        "step 5: recovery reports one interrupted transaction"
    );
    assert_eq!(
        snapshot(fixture.game()),
        step4_tree,
        "step 5: recovery restores the step 4 tree byte for byte"
    );
    assert!(
        !witness.exists(),
        "step 5: recovery removes the quarantine witness"
    );
    assert_eq!(
        assert_one_active(&fixture, "step 5 recovery"),
        "default",
        "step 5: recovery keeps the old profile active"
    );
    let doctor = fixture.json(&["doctor"]);
    assert_eq!(
        doctor["profile"]["consistent"], true,
        "step 5: doctor reports a consistent installation"
    );
    let repeated = fixture.json(&["profile", "switch", "second", "--yes"]);
    assert_eq!(
        repeated["to"], "second",
        "step 5: the repeated switch succeeds after recovery"
    );
    assert_eq!(
        bytes(&fixture, GAMMA),
        GAMMA_BYTES,
        "step 5: the repeated switch writes mod C's bytes"
    );
    assert_eq!(
        assert_one_active(&fixture, "step 5 repeated switch"),
        "second",
        "step 5: the repeated switch activates second"
    );

    // Step 6: uninstall every package in an arbitrary order. The end tree is
    // the initial tree, with only the unmanaged file and the game marker.
    uninstall(&fixture, &alpha);
    uninstall(&fixture, &gamma);
    uninstall(&fixture, &beta);
    assert_eq!(
        snapshot(fixture.game()),
        initial,
        "step 6: uninstalling every package restores the initial tree"
    );
    assert_eq!(
        fixture.json(&["list"]).as_array().expect("list").len(),
        0,
        "step 6: no managed packages remain"
    );
    assert_eq!(
        bytes(&fixture, USER),
        USER_BYTES,
        "step 6: the unmanaged file survives every uninstall"
    );
}

#[test]
fn undo_of_the_gate_switch_is_refused() {
    let fixture = ProfileFixture::new();
    install_alpha(&fixture);
    create_second(&fixture);
    let switched = fixture.json(&["profile", "switch", "second", "--yes"]);
    let operation = switched["operation_id"]
        .as_str()
        .expect("operation id")
        .to_owned();
    let before = snapshot(fixture.game());

    let (code, message) = fixture.error(&["undo", &operation, "--yes"]);

    assert_eq!(
        code, "UNDO_CONFLICT",
        "undo must refuse a profile switch: {message}"
    );
    assert!(
        message.contains("reversing a profile switch is a new switch"),
        "undo explains the switch rule: {message}"
    );
    assert_eq!(
        snapshot(fixture.game()),
        before,
        "the refused undo changes no byte"
    );
    assert_eq!(
        assert_one_active(&fixture, "undo refusal"),
        "second",
        "the refused undo keeps second active"
    );
}

#[test]
fn every_gate_command_accepts_json_and_returns_json() {
    let fixture = ProfileFixture::new();
    let alpha = install_alpha(&fixture);
    let beta = install_beta(&fixture);
    create_second(&fixture);
    let documents = vec![
        fixture.json(&["conflicts"]),
        fixture.json(&["profile", "priority", &alpha, "--rank", "1"]),
        fixture.json(&["profile", "apply", "--yes"]),
        fixture.json(&["profile", "switch", "second", "--dry-run"]),
        fixture.json(&["profile", "switch", "second", "--yes"]),
        fixture.json(&["list"]),
        fixture.json(&["profile", "list"]),
        fixture.json(&["doctor"]),
        fixture.json(&["profile", "switch", "default", "--yes"]),
        fixture.json(&["recover", "--yes"]),
        uninstall(&fixture, &beta),
        uninstall(&fixture, &alpha),
    ];

    assert_eq!(documents.len(), 12, "the gate covers 12 JSON documents");
    for (index, document) in documents.iter().enumerate() {
        assert!(
            !document.is_null(),
            "gate JSON document {index} must not be null"
        );
    }
    assert_eq!(
        documents[0]["conflicts"]
            .as_array()
            .expect("conflicts")
            .len(),
        1,
        "the first JSON document is the conflict report"
    );
    assert_eq!(
        documents[5].as_array().expect("list").len(),
        2,
        "the list JSON stays an array of packages"
    );
}

#[test]
fn profile_list_marks_exactly_one_active_profile_at_every_step() {
    let fixture = ProfileFixture::new();
    assert_eq!(
        assert_one_active(&fixture, "fresh fixture"),
        "default",
        "a fresh installation has one active profile"
    );
    let alpha = install_alpha(&fixture);
    assert_eq!(
        assert_one_active(&fixture, "after install"),
        "default",
        "installing a package keeps one active profile"
    );
    create_second(&fixture);
    assert_eq!(
        assert_one_active(&fixture, "after create"),
        "default",
        "creating a profile keeps one active profile"
    );
    fixture.json(&["profile", "switch", "second", "--yes"]);
    assert_eq!(
        assert_one_active(&fixture, "after switch"),
        "second",
        "switching keeps exactly one active profile"
    );
    fixture.json(&["profile", "switch", "default", "--yes"]);
    assert_eq!(
        assert_one_active(&fixture, "after switch back"),
        "default",
        "switching back keeps exactly one active profile"
    );
    uninstall(&fixture, &alpha);
    assert_eq!(
        assert_one_active(&fixture, "after uninstall"),
        "default",
        "uninstalling keeps exactly one active profile"
    );
}
