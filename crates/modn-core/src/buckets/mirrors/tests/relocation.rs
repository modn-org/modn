use super::*;
use modn_core::manager_roots::ManagerRoots;
use std::fs;

#[test]
fn relocation_moves_the_primary_and_the_next_sync_uses_it() {
    let key = test_key();
    let temp = tempfile::tempdir().expect("temp");
    let first = signed_zip(
        temp.path(),
        "v1",
        "1.0",
        &key,
        &[
            ("relocation.json", &relocation_json(NEW_PRIMARY)),
            ("mirrors.json", &mirrors_json(&[MIRROR])),
        ],
    );
    let state = temp.path().join("state");
    let store = BucketStore::new(&ManagerRoots::new(&state, None));
    let opener = Opener::new(vec![(PRIMARY, Some(first.clone()))]);
    let synced = store
        .sync_zip_with_signer("community", Some(zip_source(PRIMARY)), |source| {
            opener.open(source)
        })
        .expect("relocation is followed");
    assert_eq!(synced.trace.moved_from, Some(host(PRIMARY)));
    assert_eq!(synced.trace.moved_to, Some(host(NEW_PRIMARY)));
    assert_eq!(synced.revision.source.url, NEW_PRIMARY);
    let registration: serde_json::Value =
        serde_json::from_slice(&fs::read(registration_file(&state)).expect("registration"))
            .expect("JSON");
    assert_eq!(registration["zip"]["source"]["url"], NEW_PRIMARY);
    assert_eq!(
        registration["mirrors"],
        serde_json::json!([{"kind":"zip","url":MIRROR}])
    );

    let next = signed_zip(temp.path(), "v2", "2.0", &key, &[]);
    let opener = Opener::new(vec![(NEW_PRIMARY, None), (MIRROR, Some(next))]);
    store
        .sync_zip_with_signer("community", None, |source| opener.open(source))
        .expect("next sync follows the new primary");
    let opened = opener.opened();
    assert_eq!(opened[0], NEW_PRIMARY);
    assert!(!opened.contains(&PRIMARY.to_owned()), "{opened:?}");
}

#[test]
fn relocation_to_the_same_primary_is_ignored() {
    let key = test_key();
    let temp = tempfile::tempdir().expect("temp");
    let bytes = signed_zip(
        temp.path(),
        "v1",
        "1.0",
        &key,
        &[("relocation.json", &relocation_json(PRIMARY))],
    );
    let state = temp.path().join("state");
    let store = BucketStore::new(&ManagerRoots::new(&state, None));
    let opener = Opener::new(vec![(PRIMARY, Some(bytes))]);
    let synced = store
        .sync_zip_with_signer("community", Some(zip_source(PRIMARY)), |source| {
            opener.open(source)
        })
        .expect("sync");
    assert_eq!(synced.trace.moved_from, None);
    assert_eq!(synced.trace.moved_to, None);
    assert_eq!(synced.revision.source.url, PRIMARY);
}

#[test]
fn invalid_declared_sources_refuse_the_revision() {
    let key = test_key();
    let temp = tempfile::tempdir().expect("temp");
    let invalid_relocations = [
        "http://plain.example/catalog.zip",
        "C:/local/catalog.zip",
        "https://user:pass@example/catalog.zip",
    ];
    for (index, target) in invalid_relocations.iter().enumerate() {
        let state = temp.path().join(format!("relocation-state-{index}"));
        let store = BucketStore::new(&ManagerRoots::new(&state, None));
        let bytes = signed_zip(
            temp.path(),
            &format!("relocation-{index}"),
            "1.0",
            &key,
            &[("relocation.json", &relocation_json(target))],
        );
        let opener = Opener::new(vec![(PRIMARY, Some(bytes))]);
        let error = store
            .sync_zip_with_signer("community", Some(zip_source(PRIMARY)), |source| {
                opener.open(source)
            })
            .expect_err("invalid relocation is refused");
        assert!(
            format!("{error:#}").contains("invalid relocation target"),
            "{error:#}"
        );
        assert!(!registration_file(&state).exists());
    }
    let invalid_mirrors = [
        mirrors_json(&["http://plain.example/catalog.zip"]),
        mirrors_json(&["C:/local/catalog.zip"]),
        mirrors_json(&["https://user:pass@example/catalog.zip"]),
        mirrors_json(&[MIRROR, MIRROR]),
        mirrors_json(&[
            MIRROR,
            SECOND_MIRROR,
            "https://a.example/catalog.zip",
            "https://b.example/catalog.zip",
            "https://c.example/catalog.zip",
            "https://d.example/catalog.zip",
            "https://e.example/catalog.zip",
            "https://f.example/catalog.zip",
            "https://g.example/catalog.zip",
        ]),
    ];
    for (index, mirrors) in invalid_mirrors.iter().enumerate() {
        let state = temp.path().join(format!("mirrors-state-{index}"));
        let store = BucketStore::new(&ManagerRoots::new(&state, None));
        let bytes = signed_zip(
            temp.path(),
            &format!("mirrors-{index}"),
            "1.0",
            &key,
            &[("mirrors.json", mirrors)],
        );
        let opener = Opener::new(vec![(PRIMARY, Some(bytes))]);
        let error = store
            .sync_zip_with_signer("community", Some(zip_source(PRIMARY)), |source| {
                opener.open(source)
            })
            .expect_err("invalid mirrors are refused");
        assert!(format!("{error:#}").contains("mirrors.json"), "{error:#}");
        assert!(!registration_file(&state).exists());
    }
}

#[test]
fn invalid_declared_sources_keep_the_previous_registration() {
    let key = test_key();
    let temp = tempfile::tempdir().expect("temp");
    let first = signed_zip(
        temp.path(),
        "v1",
        "1.0",
        &key,
        &[("mirrors.json", &mirrors_json(&[MIRROR]))],
    );
    let state = temp.path().join("state");
    let store = BucketStore::new(&ManagerRoots::new(&state, None));
    let opener = Opener::new(vec![(PRIMARY, Some(first))]);
    store
        .sync_zip_with_signer("community", Some(zip_source(PRIMARY)), |source| {
            opener.open(source)
        })
        .expect("first sync");
    let before = state_snapshot(&state);
    let bad_mirrors = signed_zip(
        temp.path(),
        "v2",
        "2.0",
        &key,
        &[("mirrors.json", &mirrors_json(&["C:/local/catalog.zip"]))],
    );
    let opener = Opener::new(vec![(PRIMARY, Some(bad_mirrors))]);
    let error = store
        .sync_zip_with_signer("community", None, |source| opener.open(source))
        .expect_err("invalid mirrors refuse the revision");
    assert!(format!("{error:#}").contains("invalid mirror"));
    assert_eq!(before, state_snapshot(&state));

    let bad_relocation = signed_zip(
        temp.path(),
        "v3",
        "3.0",
        &key,
        &[(
            "relocation.json",
            &relocation_json("http://plain.example/catalog.zip"),
        )],
    );
    let opener = Opener::new(vec![(PRIMARY, Some(bad_relocation))]);
    let error = store
        .sync_zip_with_signer("community", None, |source| opener.open(source))
        .expect_err("invalid relocation refuses the revision");
    assert!(format!("{error:#}").contains("invalid relocation target"));
    assert_eq!(before, state_snapshot(&state));
}
