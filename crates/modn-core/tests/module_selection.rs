//! Table test for the one governing game module of a command.
//!
//! The development-override rows select the override through
//! `ManagerRoots::with_development_modules`. This test never sets
//! `MODN_GAME_MODULES_DIR`, because process environment variables are global
//! and the test runs beside other tests in the same process.

#[path = "support/catalog_git.rs"]
mod support;

#[path = "support/main_bucket.rs"]
mod main_bucket;

use modn_core::buckets::{BucketStore, ModuleSourceTrustReason, ModuleTrustAck};
use modn_core::game_modules::{ModuleOrigin, ModuleSelection, ModuleSelectionError};
use modn_core::manager_roots::ManagerRoots;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use support::{signed_remote, write_module_files};

const OFFICIAL_GAME: &str = "cyberpunk-2077";
const MODULE_GAME: &str = "fixture-game";
const MODULE_VERSION: &str = "1";

struct SelectedRow {
    name: &'static str,
    selection: ModuleSelection,
    expected_origin: ModuleOrigin,
    expected_version: &'static str,
    expected_reason: String,
}

struct RefusedRow {
    name: &'static str,
    error: ModuleSelectionError,
    expected_kind: &'static str,
    expected_message: String,
}

fn selected(
    name: &'static str,
    roots: &ManagerRoots,
    game: &str,
    expected_origin: ModuleOrigin,
    expected_version: &'static str,
    expected_reason: &str,
) -> SelectedRow {
    SelectedRow {
        name,
        selection: ModuleSelection::resolve(roots, game).expect("module selection"),
        expected_origin,
        expected_version,
        expected_reason: expected_reason.to_owned(),
    }
}

fn refused(
    name: &'static str,
    roots: &ManagerRoots,
    game: &str,
    expected_kind: &'static str,
    expected_message: &str,
) -> RefusedRow {
    RefusedRow {
        name,
        error: ModuleSelection::resolve(roots, game).expect_err("module refusal"),
        expected_kind,
        expected_message: expected_message.to_owned(),
    }
}

fn refusal_kind(error: &ModuleSelectionError) -> &'static str {
    match error {
        ModuleSelectionError::InvalidGameId => "invalid_game_id",
        ModuleSelectionError::DevelopmentOverrideRejected { .. } => "development_override_rejected",
        ModuleSelectionError::IdentityMismatch { .. } => "identity_mismatch",
        ModuleSelectionError::UntrustedModuleSource { .. } => "untrusted_module_source",
        ModuleSelectionError::NoGameModule { .. } => "no_game_module",
        ModuleSelectionError::Unavailable { .. } => "unavailable",
        _ => "unknown",
    }
}

fn untrusted_message(game: &str) -> String {
    format!(
        "game module {game} comes from bucket community, which is not a trusted module source; \
         run `modn bucket trust-modules community` to trust it again, or \
         `modn module remove {game}` to remove the module"
    )
}

fn write_module(root: &Path, game: &str, version: &str) -> PathBuf {
    let directory = root.join(format!("module-{game}-{version}"));
    fs::create_dir_all(&directory).expect("module directory");
    fs::write(
        directory.join("module.json"),
        json!({
            "game_id": game,
            "version": version,
            "api_version": 3,
            "entrypoint": "module.lua",
        })
        .to_string(),
    )
    .expect("module manifest");
    fs::write(
        directory.join("module.lua"),
        format!("return {{game_id='{game}', api_version=3}}"),
    )
    .expect("module script");
    directory
}

fn trusted_retained_release(roots: &ManagerRoots) {
    let remote = signed_remote(|root| write_module_files(root, MODULE_GAME, 3));
    let store = BucketStore::new(roots);
    store
        .sync_git_with_signer("community", Some(remote.source()))
        .expect("sync trusted catalog");
    store
        .trust_modules(
            "community",
            ModuleTrustAck::Acknowledged {
                expected_fingerprint: None,
            },
        )
        .expect("trust catalog modules");
}

fn release_receipt(roots: &ManagerRoots) -> PathBuf {
    roots
        .module_registry()
        .join(MODULE_GAME)
        .join("versions")
        .join(MODULE_VERSION)
        .join("registration.json")
}

fn recorded_signer(roots: &ManagerRoots) -> String {
    let path = release_receipt(roots);
    let receipt: Value =
        serde_json::from_slice(&fs::read(&path).expect("release receipt")).expect("receipt JSON");
    receipt["signer_key_id"]
        .as_str()
        .expect("recorded signer")
        .to_owned()
}

fn rewrite_release_signer(roots: &ManagerRoots, signer: &str) {
    let path = release_receipt(roots);
    let mut receipt: Value =
        serde_json::from_slice(&fs::read(&path).expect("release receipt")).expect("receipt JSON");
    receipt["signer_key_id"] = json!(signer);
    fs::write(&path, serde_json::to_vec(&receipt).expect("receipt JSON")).expect("write receipt");
}

#[test]
fn module_selection_table() {
    let mut selected_rows = Vec::new();
    let mut refused_rows = Vec::new();

    selected_rows.push(main_bucket::seeded_roots(|roots| {
        let dev = tempfile::tempdir().expect("development root");
        let directory = write_module(dev.path(), OFFICIAL_GAME, "9.9.9");
        let roots = roots.clone().with_development_modules(Some(directory));
        selected(
            "development_override",
            &roots,
            OFFICIAL_GAME,
            ModuleOrigin::DevelopmentOverride,
            "9.9.9",
            "the release has no bucket and is a local registration",
        )
    }));

    selected_rows.push(main_bucket::seeded_roots(|roots| {
        selected(
            "official_bucket",
            roots,
            OFFICIAL_GAME,
            ModuleOrigin::OfficialBucket,
            "0.3.0",
            "the official main bucket is always a trusted module source",
        )
    }));

    selected_rows.push(main_bucket::seeded_roots(|roots| {
        trusted_retained_release(roots);
        let reason = format!(
            "the release signer {} is authorized by the trusted signer of bucket community",
            recorded_signer(roots)
        );
        selected(
            "retained_release",
            roots,
            MODULE_GAME,
            ModuleOrigin::RetainedRelease,
            MODULE_VERSION,
            &reason,
        )
    }));

    refused_rows.push(main_bucket::seeded_roots(|roots| {
        refused(
            "invalid_game_id",
            roots,
            "Not_A_Game",
            "invalid_game_id",
            "invalid game module identifier",
        )
    }));

    refused_rows.push(main_bucket::seeded_roots(|roots| {
        let dev = tempfile::tempdir().expect("development root");
        let file = dev.path().join("not-a-module");
        fs::write(&file, b"not a module directory").expect("development override file");
        let roots = roots.clone().with_development_modules(Some(file));
        refused(
            "development_override_rejected",
            &roots,
            OFFICIAL_GAME,
            "development_override_rejected",
            "module directory must be a real directory (not a link)",
        )
    }));

    refused_rows.push(main_bucket::seeded_roots(|roots| {
        let dev = tempfile::tempdir().expect("development root");
        let directory = write_module(dev.path(), "other-game", "9.9.9");
        let roots = roots.clone().with_development_modules(Some(directory));
        refused(
            "identity_mismatch",
            &roots,
            OFFICIAL_GAME,
            "identity_mismatch",
            "game module identity does not match requested game",
        )
    }));

    refused_rows.push(main_bucket::seeded_roots(|roots| {
        trusted_retained_release(roots);
        BucketStore::new(roots)
            .untrust_modules("community")
            .expect("untrust catalog modules");
        let row = refused(
            "retained_release_from_an_untrusted_bucket",
            roots,
            MODULE_GAME,
            "untrusted_module_source",
            &untrusted_message(MODULE_GAME),
        );
        match &row.error {
            ModuleSelectionError::UntrustedModuleSource {
                bucket, decision, ..
            } => {
                assert_eq!(bucket, "community", "{}", row.name);
                assert!(
                    matches!(
                        decision.reason(),
                        ModuleSourceTrustReason::NoTrustRecord { bucket } if bucket == "community"
                    ),
                    "{}: {:?}",
                    row.name,
                    decision.reason()
                );
            }
            other => panic!("{} refused as {other:?}", row.name),
        }
        row
    }));

    refused_rows.push(main_bucket::seeded_roots(|roots| {
        trusted_retained_release(roots);
        rewrite_release_signer(roots, "0123456789ABCDEF");
        let row = refused(
            "retained_release_with_a_different_signer",
            roots,
            MODULE_GAME,
            "untrusted_module_source",
            &untrusted_message(MODULE_GAME),
        );
        match &row.error {
            ModuleSelectionError::UntrustedModuleSource {
                bucket, decision, ..
            } => {
                assert_eq!(bucket, "community", "{}", row.name);
                assert!(
                    matches!(
                        decision.reason(),
                        ModuleSourceTrustReason::UnauthorizedReleaseSigner { signer, .. }
                            if signer == "0123456789ABCDEF"
                    ),
                    "{}: {:?}",
                    row.name,
                    decision.reason()
                );
            }
            other => panic!("{} refused as {other:?}", row.name),
        }
        row
    }));

    refused_rows.push(main_bucket::seeded_roots(|roots| {
        let expected = format!(
            "no first-party game module for no-such-game; expected {}",
            roots
                .official_main()
                .join("games")
                .join("no-such-game")
                .display()
        );
        refused(
            "unknown_game",
            roots,
            "no-such-game",
            "no_game_module",
            &expected,
        )
    }));

    assert_eq!(selected_rows.len(), 3, "the table covers every origin");
    assert_eq!(refused_rows.len(), 6, "the table covers every refusal");
    for row in selected_rows {
        assert_eq!(row.selection.origin(), row.expected_origin, "{}", row.name);
        assert_eq!(
            row.selection.module().game_id(),
            if row.expected_origin == ModuleOrigin::RetainedRelease {
                MODULE_GAME
            } else {
                OFFICIAL_GAME
            },
            "{}",
            row.name
        );
        assert_eq!(
            row.selection.module().version(),
            row.expected_version,
            "{}",
            row.name
        );
        assert!(row.selection.trust().is_trusted(), "{}", row.name);
        assert_eq!(
            row.selection.trust().reason().to_string(),
            row.expected_reason,
            "{}",
            row.name
        );
        assert!(!row.selection.script().is_empty(), "{}", row.name);
    }
    for row in refused_rows {
        assert_eq!(refusal_kind(&row.error), row.expected_kind, "{}", row.name);
        assert_eq!(row.error.to_string(), row.expected_message, "{}", row.name);
    }
}
