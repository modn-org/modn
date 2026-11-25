use super::tests::{archive, package_with_sources, FALLBACK_URL};
use super::SuppliedArchive;
use modn_core::buckets::PackageSource;
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn a_supplied_archive_that_does_not_match_the_pin_stays_manual() {
    let temporary = tempfile::tempdir().expect("fixture");
    let root = temporary.path().join("manager");
    std::fs::create_dir_all(&root).expect("manager directory");
    archive(temporary.path());
    let package = package_with_sources(
        &root,
        &"0".repeat(64),
        json!([{"kind": "https", "url": FALLBACK_URL}]),
    );
    let reference = crate::catalog_install::reference_of(&package).to_string();
    let supplied = BTreeMap::from([(
        reference.clone(),
        SuppliedArchive {
            path: temporary.path().join("example.zip"),
            blake3: "1".repeat(64),
        },
    )]);
    let source = super::serving_source(&reference, &supplied, &BTreeMap::new(), &package);
    assert!(
        matches!(source, PackageSource::Manual { .. }),
        "a file that is not the pinned archive must stay manual: {source:?}"
    );
}

#[test]
fn a_matching_supplied_digest_names_the_catalog_source_without_reading_the_file() {
    let temporary = tempfile::tempdir().expect("fixture");
    let root = temporary.path().join("manager");
    std::fs::create_dir_all(&root).expect("manager directory");
    let (_, digest) = archive(temporary.path());
    let package = package_with_sources(
        &root,
        &digest,
        json!([{"kind": "https", "url": FALLBACK_URL}]),
    );
    let reference = crate::catalog_install::reference_of(&package).to_string();
    let supplied = BTreeMap::from([(
        reference.clone(),
        SuppliedArchive {
            path: temporary.path().join("moved-away.zip"),
            blake3: digest,
        },
    )]);
    let source = super::serving_source(&reference, &supplied, &BTreeMap::new(), &package);
    assert!(
        matches!(source, PackageSource::Https { .. }),
        "a verified file must record the catalog source: {source:?}"
    );
}
