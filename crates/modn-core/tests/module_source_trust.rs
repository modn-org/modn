#[path = "support/catalog_git.rs"]
mod support;

#[path = "support/main_bucket.rs"]
mod main_bucket;

use modn_core::buckets::{
    decide_module_source_trust, BucketStore, ModuleSourceBucketKind, ModuleSourceRegistrationFacts,
    ModuleSourceTrustDecision, ModuleTrustAck,
};
use modn_core::game_modules::ModuleSource;
use modn_core::manager_roots::ManagerRoots;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use support::{registration, signed_remote, write_module_files};

struct Row {
    name: &'static str,
    decision: ModuleSourceTrustDecision,
    trusted: bool,
    reason: String,
    remedy: Option<(String, String)>,
}

fn expected_remedy(bucket: &str) -> (String, String) {
    (
        format!("modn bucket trust-modules {bucket}"),
        "modn module remove test-game".to_owned(),
    )
}

fn trusted_row(name: &'static str, decision: ModuleSourceTrustDecision, reason: &str) -> Row {
    assert!(decision.is_trusted(), "{name}");
    Row {
        name,
        decision,
        trusted: true,
        reason: reason.to_owned(),
        remedy: None,
    }
}

fn refused_row(
    name: &'static str,
    decision: ModuleSourceTrustDecision,
    reason: String,
    bucket: &str,
) -> Row {
    assert!(!decision.is_trusted(), "{name}");
    Row {
        name,
        decision,
        trusted: false,
        reason,
        remedy: Some(expected_remedy(bucket)),
    }
}

fn empty_store() -> (tempfile::TempDir, BucketStore) {
    let state = tempfile::tempdir().expect("state");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main");
    main_bucket::copy_tree(&source, &state.path().join("buckets/main"));
    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));
    (state, store)
}

fn local_bucket(root: &Path) -> PathBuf {
    let bucket = root.join("local-bucket");
    fs::create_dir_all(bucket.join("games/test-game")).expect("games");
    fs::write(
        bucket.join("bucket.json"),
        json!({"schema_version": 2, "name": "local", "title": "Local"}).to_string(),
    )
    .expect("metadata");
    bucket
}

fn facts(
    trust_record_signer: Option<&str>,
    pinned_signer: Option<&str>,
    previous_signers: &[&str],
    revoked_signers: &[&str],
) -> ModuleSourceRegistrationFacts {
    ModuleSourceRegistrationFacts {
        kind: ModuleSourceBucketKind::Catalog,
        trust_record_signer: trust_record_signer.map(str::to_owned),
        pinned_signer: pinned_signer.map(str::to_owned),
        previous_signers: previous_signers
            .iter()
            .map(|signer| (*signer).to_owned())
            .collect(),
        revoked_signers: revoked_signers
            .iter()
            .map(|signer| (*signer).to_owned())
            .collect(),
    }
}

fn source(bucket: Option<&str>, signer: Option<&str>) -> ModuleSource {
    ModuleSource {
        bucket: bucket.map(str::to_owned),
        signer_key_id: signer.map(str::to_owned),
    }
}

#[test]
fn module_source_trust_table() {
    let mut rows = Vec::new();

    {
        let (_state, store) = empty_store();
        let decision = store
            .module_source_trust(&source(Some("main"), None))
            .expect("decision");
        rows.push(trusted_row(
            "main_is_always_trusted",
            decision,
            "the official main bucket is always a trusted module source",
        ));
    }

    {
        let (_state, store) = empty_store();
        let decision = store
            .module_source_trust(&source(None, None))
            .expect("decision");
        rows.push(trusted_row(
            "a_release_without_a_bucket_is_a_local_registration",
            decision,
            "the release has no bucket and is a local registration",
        ));
    }

    {
        let (state, store) = empty_store();
        let bucket = local_bucket(state.path());
        store.add("local", &bucket).expect("add local bucket");
        let decision = store
            .module_source_trust(&source(Some("local"), None))
            .expect("decision");
        rows.push(refused_row(
            "a_local_folder_bucket_is_refused",
            decision,
            "local folder bucket local cannot supply game modules; only the official main bucket and signed Git catalogs the user trusts for modules supply game modules".to_owned(),
            "local",
        ));
    }

    {
        let (_state, store) = empty_store();
        let remote = signed_remote(|root| write_module_files(root, "test-game", 3));
        let revision = store
            .sync_git_with_signer("community", Some(remote.source()))
            .expect("sync")
            .revision;
        let decision = store
            .module_source_trust(&source(Some("community"), Some(&revision.signer.key_id)))
            .expect("decision");
        rows.push(refused_row(
            "a_git_catalog_without_a_trust_record_is_refused",
            decision,
            "bucket community has no module trust record".to_owned(),
            "community",
        ));
    }

    {
        let (state, store) = empty_store();
        let remote = signed_remote(|root| write_module_files(root, "test-game", 3));
        let revision = store
            .sync_git_with_signer("community", Some(remote.source()))
            .expect("sync")
            .revision;
        let path = registration(state.path());
        let mut stored: Value =
            serde_json::from_slice(&fs::read(&path).expect("registration")).expect("JSON");
        stored["git"].as_object_mut().expect("git").remove("signer");
        fs::write(&path, serde_json::to_vec(&stored).expect("JSON")).expect("write");
        let decision = store
            .module_source_trust(&source(Some("community"), Some(&revision.signer.key_id)))
            .expect("decision");
        rows.push(refused_row(
            "a_git_catalog_without_a_pinned_signer_is_refused",
            decision,
            "bucket community pins no catalog signer".to_owned(),
            "community",
        ));
    }

    {
        let (_state, store) = empty_store();
        let remote = signed_remote(|root| write_module_files(root, "test-game", 3));
        let revision = store
            .sync_git_with_signer("community", Some(remote.source()))
            .expect("sync")
            .revision;
        store
            .trust_modules(
                "community",
                ModuleTrustAck::Acknowledged {
                    expected_fingerprint: None,
                },
            )
            .expect("trust");
        let decision = store
            .module_source_trust(&source(Some("community"), Some(&revision.signer.key_id)))
            .expect("decision");
        rows.push(trusted_row(
            "a_trusted_git_catalog_is_trusted",
            decision,
            &format!(
                "the release signer {} is authorized by the trusted signer of bucket community",
                revision.signer.key_id
            ),
        ));
    }

    {
        let (_state, store) = empty_store();
        let remote = signed_remote(|root| write_module_files(root, "test-game", 3));
        let revision = store
            .sync_git_with_signer("community", Some(remote.source()))
            .expect("sync")
            .revision;
        store
            .trust_modules(
                "community",
                ModuleTrustAck::Acknowledged {
                    expected_fingerprint: None,
                },
            )
            .expect("trust");
        let decision = store
            .module_source_trust(&source(Some("community"), Some("0123456789ABCDEF")))
            .expect("decision");
        rows.push(refused_row(
            "a_trusted_catalog_refuses_a_different_recorded_signer",
            decision,
            format!(
                "the release signer 0123456789ABCDEF is not authorized by the trusted signer {} of bucket community",
                revision.signer.key_id
            ),
            "community",
        ));
    }

    {
        let (_state, store) = empty_store();
        let decision = store
            .module_source_trust(&source(Some("missing"), None))
            .expect("decision");
        rows.push(refused_row(
            "an_unregistered_bucket_is_refused",
            decision,
            "bucket missing is not registered".to_owned(),
            "missing",
        ));
    }

    rows.push(refused_row(
        "a_trust_record_for_an_older_signer_is_refused",
        decide_module_source_trust(
            &source(Some("community"), Some("B4B4B4B4B4B4B4B4")),
            Some(&facts(
                Some("A1A1A1A1A1A1A1A1"),
                Some("B4B4B4B4B4B4B4B4"),
                &[],
                &[],
            )),
        ),
        "bucket community was trusted for signer A1A1A1A1A1A1A1A1, but its pinned signer is B4B4B4B4B4B4B4B4".to_owned(),
        "community",
    ));

    rows.push(refused_row(
        "a_release_without_a_recorded_signer_is_refused",
        decide_module_source_trust(
            &source(Some("community"), None),
            Some(&facts(
                Some("B4B4B4B4B4B4B4B4"),
                Some("B4B4B4B4B4B4B4B4"),
                &[],
                &[],
            )),
        ),
        "the release from bucket community records no signer".to_owned(),
        "community",
    ));

    rows.push(trusted_row(
        "a_rotation_keeps_a_previous_signer_authorized",
        decide_module_source_trust(
            &source(Some("community"), Some("A1A1A1A1A1A1A1A1")),
            Some(&facts(
                Some("B4B4B4B4B4B4B4B4"),
                Some("B4B4B4B4B4B4B4B4"),
                &["A1A1A1A1A1A1A1A1"],
                &[],
            )),
        ),
        "the release signer A1A1A1A1A1A1A1A1 is authorized by the trusted signer of bucket community",
    ));

    rows.push(refused_row(
        "a_revocation_refuses_a_revoked_signer",
        decide_module_source_trust(
            &source(Some("community"), Some("A1A1A1A1A1A1A1A1")),
            Some(&facts(
                Some("B4B4B4B4B4B4B4B4"),
                Some("B4B4B4B4B4B4B4B4"),
                &["A1A1A1A1A1A1A1A1"],
                &["A1A1A1A1A1A1A1A1"],
            )),
        ),
        "the release signer A1A1A1A1A1A1A1A1 was revoked by bucket community".to_owned(),
        "community",
    ));

    assert!(rows.len() >= 12, "the table keeps every case");
    for row in rows {
        assert_eq!(row.decision.is_trusted(), row.trusted, "{}", row.name);
        assert_eq!(
            row.decision.reason().to_string(),
            row.reason,
            "{}",
            row.name
        );
        match row.decision.remedy() {
            Some(remedy) => {
                let expected = row.remedy.expect("refused rows carry a remedy");
                assert_eq!(remedy.trust_modules_command(), expected.0, "{}", row.name);
                assert_eq!(
                    remedy.remove_module_command("test-game"),
                    expected.1,
                    "{}",
                    row.name
                );
            }
            None => assert!(
                row.remedy.is_none(),
                "{} is trusted and carries no remedy",
                row.name
            ),
        }
    }
}

#[test]
fn remedy_text_orders_commands_by_refusal_reason() {
    let signer_change = decide_module_source_trust(
        &source(Some("community"), Some("B4B4B4B4B4B4B4B4")),
        Some(&facts(
            Some("A1A1A1A1A1A1A1A1"),
            Some("B4B4B4B4B4B4B4B4"),
            &[],
            &[],
        )),
    );
    assert_eq!(
        signer_change.remedy_text("test-game").expect("remedy"),
        "Run `modn module remove test-game` first, then `modn bucket trust-modules community` to trust the bucket's current signer and import the module again."
    );

    // The release-signer mismatch left by a plain signer change needs the same
    // order: re-trusting does not rewrite the retained release's signer.
    let unauthorized_signer = decide_module_source_trust(
        &source(Some("community"), Some("C5C5C5C5C5C5C5C5")),
        Some(&facts(
            Some("B4B4B4B4B4B4B4B4"),
            Some("B4B4B4B4B4B4B4B4"),
            &[],
            &[],
        )),
    );
    assert_eq!(
        unauthorized_signer.remedy_text("test-game").expect("remedy"),
        "Run `modn module remove test-game` first, then `modn bucket trust-modules community` to trust the bucket's current signer and import the module again."
    );

    let unregistered = decide_module_source_trust(&source(Some("missing"), None), None);
    let text = unregistered.remedy_text("test-game").expect("remedy");
    assert!(
        !text.contains("trust-modules"),
        "an unregistered bucket cannot be trusted with trust-modules: {text}"
    );
    assert!(
        text.contains("modn module remove test-game"),
        "the unregistered-bucket remedy must remove the module: {text}"
    );
    assert!(
        text.contains("modn bucket sync missing --source <remote> --revision <reference>"),
        "the unregistered-bucket remedy must name the registration command: {text}"
    );

    let no_trust_record = decide_module_source_trust(
        &source(Some("community"), Some("B4B4B4B4B4B4B4B4")),
        Some(&facts(None, Some("B4B4B4B4B4B4B4B4"), &[], &[])),
    );
    assert_eq!(
        no_trust_record.remedy_text("test-game").expect("remedy"),
        "Run `modn bucket trust-modules community` to trust it again, or `modn module remove test-game` to remove the module."
    );

    let trusted = decide_module_source_trust(&source(Some("main"), None), None);
    assert_eq!(trusted.remedy_text("test-game"), None);
}
