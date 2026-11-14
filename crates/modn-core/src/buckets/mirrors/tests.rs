use super::*;
use crate::buckets::{catalog_statement, BucketStore, ZipCatalogSource};
use anyhow::{bail, Result};
use minisign::KeyPair;
use std::cell::RefCell;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

const PRIMARY: &str = "https://primary.example/catalog.zip";
const MIRROR: &str = "https://mirror.example/catalog.zip";
const SECOND_MIRROR: &str = "https://second.example/catalog.zip";
const NEW_PRIMARY: &str = "https://new.example/catalog.zip";

struct TestKey {
    pair: KeyPair,
    text: String,
}

fn test_key() -> TestKey {
    let pair = KeyPair::generate_unencrypted_keypair().expect("test key");
    let text = pair.pk.to_box().expect("public key").into_string();
    TestKey { pair, text }
}

fn package(version: &str) -> String {
    serde_json::json!({"schema_version":2,"id":"mod","game":"test-game","name":"Mod",
        "version":version,"archive_format":"zip","blake3":"0".repeat(64),
        "sources":[{"kind":"manual","filename":"mod.zip"}]})
    .to_string()
}

fn seed_catalog(root: &Path, version: &str) {
    fs::create_dir_all(root).expect("catalog directory");
    fs::write(
        root.join("bucket.json"),
        r#"{"schema_version":2,"name":"community","title":"Community"}"#,
    )
    .expect("bucket.json");
    fs::create_dir_all(root.join("mods/test-game")).expect("mods directory");
    fs::write(root.join("mods/test-game/mod.json"), package(version)).expect("package");
}

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("target directory");
    for entry in fs::read_dir(source).expect("entries") {
        let entry = entry.expect("entry");
        let target = target.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy");
        }
    }
}

fn write_signature(root: &Path, key: &TestKey) {
    let staging = tempfile::tempdir().expect("staging");
    copy_tree(root, staging.path());
    let statement = catalog_statement(staging.path()).expect("statement");
    let signature = minisign::sign(
        Some(&key.pair.pk),
        &key.pair.sk,
        Cursor::new(statement.as_bytes()),
        Some("catalog test"),
        Some("catalog test signature"),
    )
    .expect("sign")
    .into_string();
    fs::write(root.join("catalog.pub"), &key.text).expect("catalog.pub");
    fs::write(root.join("catalog.sig"), signature).expect("catalog.sig");
}

fn pack(root: &Path, zip: &Path) {
    fn collect(root: &Path, current: &Path, output: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(current).expect("entries") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                collect(root, &path, output);
            } else {
                output.push((
                    path.strip_prefix(root)
                        .expect("relative")
                        .to_str()
                        .expect("UTF-8")
                        .replace('\\', "/"),
                    fs::read(path).expect("bytes"),
                ));
            }
        }
    }
    let mut entries = Vec::new();
    collect(root, root, &mut entries);
    entries.sort();
    let mut writer = zip::ZipWriter::new(fs::File::create(zip).expect("ZIP"));
    for (name, bytes) in entries {
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .expect("entry");
        writer.write_all(&bytes).expect("bytes");
    }
    writer.finish().expect("complete");
}

/// A signed catalog tree plus its packed ZIP bytes.
fn signed_zip(root: &Path, name: &str, version: &str, key: &TestKey, extra: &[(&str, &str)]) -> Vec<u8> {
    let tree = root.join(format!("{name}-tree"));
    seed_catalog(&tree, version);
    for (file, body) in extra {
        fs::write(tree.join(file), body).expect("catalog metadata");
    }
    write_signature(&tree, key);
    let zip = root.join(format!("{name}.zip"));
    pack(&tree, &zip);
    fs::read(zip).expect("ZIP bytes")
}

fn host(url: &str) -> CatalogHost {
    CatalogHost::Zip {
        url: url.to_owned(),
    }
}

fn mirrors_json(urls: &[&str]) -> String {
    let mirrors: Vec<serde_json::Value> = urls
        .iter()
        .map(|url| serde_json::json!({"kind":"zip","url":url}))
        .collect();
    serde_json::json!({"schema_version":1,"mirrors":mirrors}).to_string()
}

fn relocation_json(url: &str) -> String {
    serde_json::json!({"schema_version":1,"primary":{"kind":"zip","url":url}}).to_string()
}

fn zip_source(url: &str) -> ZipCatalogSource {
    ZipCatalogSource::new(url.to_owned())
}

fn registration_file(state: &Path) -> PathBuf {
    state.join("buckets/registrations/community.json")
}

fn published(state: &Path) -> PathBuf {
    let bytes = fs::read(registration_file(state)).expect("registration");
    let value: serde_json::Value = serde_json::from_slice(&bytes).expect("registration JSON");
    PathBuf::from(value["path"].as_str().expect("published path"))
}

fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn collect(root: &Path, current: &Path, output: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(current).expect("entries") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                collect(root, &path, output);
            } else {
                output.push((
                    path.strip_prefix(root)
                        .expect("relative")
                        .to_string_lossy()
                        .replace('\\', "/"),
                    fs::read(path).expect("bytes"),
                ));
            }
        }
    }
    let mut output = Vec::new();
    collect(root, root, &mut output);
    output.sort();
    output
}

fn state_snapshot(state: &Path) -> (Vec<u8>, Vec<(String, Vec<u8>)>) {
    (
        fs::read(registration_file(state)).expect("registration"),
        snapshot(&published(state)),
    )
}

struct Opener {
    opened: Rc<RefCell<Vec<String>>>,
    responses: Vec<(String, Option<Vec<u8>>)>,
}

impl Opener {
    fn new(responses: Vec<(&str, Option<Vec<u8>>)>) -> Self {
        Self {
            opened: Rc::new(RefCell::new(Vec::new())),
            responses: responses
                .into_iter()
                .map(|(url, bytes)| (url.to_owned(), bytes))
                .collect(),
        }
    }

    fn opened(&self) -> Vec<String> {
        self.opened.borrow().clone()
    }
}

impl Opener {
    fn open(&self, source: &ZipCatalogSource) -> Result<Box<dyn Read>> {
        self.opened.borrow_mut().push(source.url.clone());
        match self
            .responses
            .iter()
            .find(|(url, _)| url == &source.url)
            .and_then(|(_, bytes)| bytes.as_ref())
        {
            Some(bytes) => Ok(Box::new(Cursor::new(bytes.clone()))),
            None => bail!("connection refused"),
        }
    }
}

#[test]
fn mirror_serves_the_revision_and_the_primary_stays_registered() {
    let key = test_key();
    let temp = tempfile::tempdir().expect("temp");
    let first = signed_zip(
        temp.path(),
        "v1",
        "1.0",
        &key,
        &[("mirrors.json", &mirrors_json(&[MIRROR, SECOND_MIRROR]))],
    );
    let state = temp.path().join("state");
    let store = BucketStore::new(&state);
    let opener = Opener::new(vec![(PRIMARY, Some(first))]);
    store
        .sync_zip_with_signer("community", Some(zip_source(PRIMARY)), |source| {
            opener.open(source)
        })
        .expect("first sync registers the mirrors");

    // The primary fails; the first stored mirror serves a later revision.
    let second = signed_zip(
        temp.path(),
        "v2",
        "2.0",
        &key,
        &[("mirrors.json", &mirrors_json(&[SECOND_MIRROR]))],
    );
    let failing = Opener::new(vec![
        (PRIMARY, None),
        (MIRROR, Some(second)),
        (SECOND_MIRROR, Some(Vec::new())),
    ]);
    let synced = store
        .sync_zip_with_signer("community", None, |source| failing.open(source))
        .expect("mirror serves the revision");
    assert_eq!(synced.trace.primary, host(PRIMARY));
    assert_eq!(synced.trace.served_by, host(MIRROR));
    assert!(synced.trace.failed_sources[0].contains(PRIMARY));
    assert!(synced.trace.failed_sources[0].contains("connection refused"));
    assert_eq!(synced.revision.source.url, PRIMARY);
    assert_eq!(failing.opened(), vec![PRIMARY, MIRROR]);
    let registration: serde_json::Value =
        serde_json::from_slice(&fs::read(registration_file(&state)).expect("registration"))
            .expect("JSON");
    assert_eq!(registration["zip"]["source"]["url"], PRIMARY);
    // The verified revision's mirror list replaces the stored list.
    assert_eq!(
        registration["mirrors"],
        serde_json::json!([{"kind":"zip","url":SECOND_MIRROR}])
    );
}

#[test]
fn revision_signed_by_another_key_stops_before_any_mirror() {
    let key = test_key();
    let other = test_key();
    let temp = tempfile::tempdir().expect("temp");
    let first = signed_zip(
        temp.path(),
        "v1",
        "1.0",
        &key,
        &[("mirrors.json", &mirrors_json(&[MIRROR]))],
    );
    let state = temp.path().join("state");
    let store = BucketStore::new(&state);
    let opener = Opener::new(vec![(PRIMARY, Some(first))]);
    store
        .sync_zip_with_signer("community", Some(zip_source(PRIMARY)), |source| {
            opener.open(source)
        })
        .expect("first sync");
    let before = state_snapshot(&state);

    let rekeyed = signed_zip(
        temp.path(),
        "v2",
        "2.0",
        &other,
        &[
            ("mirrors.json", &mirrors_json(&[MIRROR])),
            ("relocation.json", &relocation_json(NEW_PRIMARY)),
        ],
    );
    let opener = Opener::new(vec![(PRIMARY, Some(rekeyed)), (MIRROR, Some(Vec::new()))]);
    let error = store
        .sync_zip_with_signer("community", None, |source| opener.open(source))
        .expect_err("foreign signer is refused");
    let message = format!("{error:#}");
    assert!(message.contains(PRIMARY), "{message}");
    assert!(message.contains("Signer key changed"), "{message}");
    assert_eq!(opener.opened(), vec![PRIMARY]);
    assert_eq!(before, state_snapshot(&state));
}

#[test]
fn tampered_and_unsigned_revisions_are_refused_without_trying_mirrors() {
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
    let store = BucketStore::new(&state);
    let opener = Opener::new(vec![(PRIMARY, Some(first))]);
    store
        .sync_zip_with_signer("community", Some(zip_source(PRIMARY)), |source| {
            opener.open(source)
        })
        .expect("first sync");
    let before = state_snapshot(&state);

    let tree = temp.path().join("tampered-tree");
    copy_tree(&temp.path().join("v1-tree"), &tree);
    let record = tree.join("mods/test-game/mod.json");
    fs::write(&record, package("9.0")).expect("tampered record");
    let zip = temp.path().join("tampered.zip");
    pack(&tree, &zip);
    let tampered = fs::read(&zip).expect("ZIP");
    let opener = Opener::new(vec![(PRIMARY, Some(tampered)), (MIRROR, Some(Vec::new()))]);
    let error = store
        .sync_zip_with_signer("community", None, |source| opener.open(source))
        .expect_err("tampered revision is refused");
    assert!(format!("{error:#}").contains("catalog signature verification failed"));
    assert_eq!(opener.opened(), vec![PRIMARY]);
    assert_eq!(before, state_snapshot(&state));

    let unsigned_tree = temp.path().join("unsigned-tree");
    copy_tree(&temp.path().join("v1-tree"), &unsigned_tree);
    fs::remove_file(unsigned_tree.join("catalog.sig")).expect("remove signature");
    let zip = temp.path().join("unsigned.zip");
    pack(&unsigned_tree, &zip);
    let unsigned = fs::read(&zip).expect("ZIP");
    let opener = Opener::new(vec![(PRIMARY, Some(unsigned)), (MIRROR, Some(Vec::new()))]);
    let error = store
        .sync_zip_with_signer("community", None, |source| opener.open(source))
        .expect_err("unsigned revision is refused");
    assert!(format!("{error:#}").contains("unsigned"));
    assert_eq!(opener.opened(), vec![PRIMARY]);
    assert_eq!(before, state_snapshot(&state));
}

#[test]
fn every_failed_source_is_listed_and_state_is_unchanged() {
    let key = test_key();
    let temp = tempfile::tempdir().expect("temp");
    let first = signed_zip(
        temp.path(),
        "v1",
        "1.0",
        &key,
        &[("mirrors.json", &mirrors_json(&[MIRROR, SECOND_MIRROR]))],
    );
    let state = temp.path().join("state");
    let store = BucketStore::new(&state);
    let opener = Opener::new(vec![(PRIMARY, Some(first))]);
    store
        .sync_zip_with_signer("community", Some(zip_source(PRIMARY)), |source| {
            opener.open(source)
        })
        .expect("first sync");
    let before = state_snapshot(&state);

    let failing = Opener::new(vec![
        (PRIMARY, None),
        (MIRROR, None),
        (SECOND_MIRROR, None),
    ]);
    let error = store
        .sync_zip_with_signer("community", None, |source| failing.open(source))
        .expect_err("all sources fail");
    let message = format!("{error:#}");
    for url in [PRIMARY, MIRROR, SECOND_MIRROR] {
        assert!(message.contains(url), "{message}");
    }
    assert_eq!(message.matches("connection refused").count(), 3, "{message}");
    assert_eq!(before, state_snapshot(&state));
}

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
    let store = BucketStore::new(&state);
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
    let store = BucketStore::new(&state);
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
        let store = BucketStore::new(&state);
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
        let store = BucketStore::new(&state);
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
        assert!(
            format!("{error:#}").contains("mirrors.json"),
            "{error:#}"
        );
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
    let store = BucketStore::new(&state);
    let opener = Opener::new(vec![(PRIMARY, Some(first))]);
    store
        .sync_zip_with_signer("community", Some(zip_source(PRIMARY)), |source| {
            opener.open(source)
        })
        .expect("first sync");
    let before = state_snapshot(&state);
    let bad = signed_zip(
        temp.path(),
        "v2",
        "2.0",
        &key,
        &[("mirrors.json", &mirrors_json(&["C:/local/catalog.zip"]))],
    );
    let opener = Opener::new(vec![(PRIMARY, Some(bad))]);
    let error = store
        .sync_zip_with_signer("community", None, |source| opener.open(source))
        .expect_err("invalid mirrors refuse the revision");
    assert!(format!("{error:#}").contains("invalid mirror"));
    assert_eq!(before, state_snapshot(&state));
}

#[test]
fn failed_registration_write_keeps_the_previous_state() {
    let key = test_key();
    let temp = tempfile::tempdir().expect("temp");
    let first = signed_zip(temp.path(), "v1", "1.0", &key, &[]);
    let state = temp.path().join("state");
    let store = BucketStore::new(&state);
    let opener = Opener::new(vec![(PRIMARY, Some(first))]);
    store
        .sync_zip_with_signer("community", Some(zip_source(PRIMARY)), |source| {
            opener.open(source)
        })
        .expect("first sync");
    let before = state_snapshot(&state);
    let second = signed_zip(temp.path(), "v2", "2.0", &key, &[]);
    store
        .fail_next_registration_write
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let opener = Opener::new(vec![(PRIMARY, Some(second))]);
    let error = store
        .sync_zip_with_signer("community", None, |source| opener.open(source))
        .expect_err("write failure");
    assert!(format!("{error:#}").contains("injected registration write failure"));
    assert_eq!(before, state_snapshot(&state));
}
