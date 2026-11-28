//! Catalog installs over a file another managed package owns.

use super::commit_support::{digest, manifest, CommitFixture, FakeTransport, FetchResolver};
use crate::acquisition::{CacheUse, SourcePolicy};
use crate::catalog_pipeline::{commit_plan, CatalogPlan, CommitOutcome, CommitRequest, Consent};
use crate::progress::OperationControl;
use crate::state::Store;
use std::fs;

fn commit(
    fixture: &CommitFixture,
    store: &mut Store,
    plan: &CatalogPlan,
    consent: &Consent,
) -> anyhow::Result<CommitOutcome> {
    commit_plan(CommitRequest::new(
        fixture.roots(),
        &fixture.cache(),
        plan,
        consent,
        &FakeTransport::default(),
        &FetchResolver,
        SourcePolicy::default(),
        CacheUse::Reuse,
        &OperationControl::default(),
        store,
    ))
}

#[test]
fn override_catalog_install_replaces_a_managed_file_and_records_the_claim() {
    let fixture = CommitFixture::new();
    let base = fixture.versioned_plugin("base.zip", "shared", "first-bytes");
    let top = fixture.versioned_plugin("top.zip", "shared", "second-bytes");
    fixture.add(&manifest("base", Some(&digest(&base))));
    fixture.add(&manifest("top", Some(&digest(&top))));
    let snapshot = fixture.snapshot();
    let mut store = fixture.open_store();
    let path = fixture.game().join("addons/shared.plugin");

    let base_plan = fixture.plan(&snapshot, "base", None, std::slice::from_ref(&base));
    let base_previews = fixture.preview(&base_plan, &fixture.local_archives(&[("base", &base)]));
    let base_consent = fixture.consent(&base_plan, &base_previews);
    let first = commit(&fixture, &mut store, &base_plan, &base_consent).expect("base install");
    assert_eq!(fs::read(&path).expect("bytes"), b"first-bytes");

    let top_plan = fixture.plan(&snapshot, "top", Some(&store), std::slice::from_ref(&top));
    let top_previews = fixture.preview_with(
        Some(&store),
        &top_plan,
        &fixture.local_archives(&[("top", &top)]),
    );
    assert_eq!(top_previews.len(), 1);
    assert_eq!(top_previews[0].plan.overrides.len(), 1, "{top_previews:?}");
    assert_eq!(
        top_previews[0].plan.overrides[0].destination,
        "addons/shared.plugin"
    );
    assert_eq!(
        top_previews[0].plan.overrides[0].replaced_package,
        first.packages[0].id
    );
    let top_consent = fixture.consent(&top_plan, &top_previews);
    let second = commit(&fixture, &mut store, &top_plan, &top_consent).expect("override install");

    assert_eq!(fs::read(&path).expect("bytes"), b"second-bytes");
    assert_eq!(fixture.count("file_claims"), 2);
    let winner = winner_of(&store, "addons/shared.plugin");
    assert_eq!(winner, second.packages[0].id);
}

fn winner_of(store: &Store, destination: &str) -> String {
    store
        .connection
        .query_row(
            "SELECT package_id FROM managed_files WHERE relative_path=?1",
            [destination],
            |row| row.get(0),
        )
        .expect("winner row")
}
