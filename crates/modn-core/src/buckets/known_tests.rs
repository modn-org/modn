use super::{KnownBuckets, KNOWN_BUCKETS_FILE};
use crate::buckets::{BucketStore, CatalogKeyId};
use crate::manager_roots::ManagerRoots;
use minisign::KeyPair;
use std::fs;
use std::io::{self, Cursor, Write};
use std::path::{Path, PathBuf};

const VALID: &str = r#"{
  "schema_version": 1,
  "buckets": {
    "zeta": {
      "kind": "zip",
      "source": "https://mirror.example/zeta.zip",
      "fingerprint": "abcdef0123456789",
      "description": "Zeta bucket."
    },
    "alpha": {
      "kind": "git",
      "source": "https://github.com/owner/alpha-bucket",
      "revision": "main",
      "fingerprint": "0123456789ABCDEF",
      "description": "Alpha bucket."
    }
  }
}"#;

fn parse_file(text: &str) -> anyhow::Result<KnownBuckets> {
    KnownBuckets::parse(text.as_bytes())
}

fn entry(
    kind: &str,
    source: &str,
    revision: Option<&str>,
    fingerprint: &str,
    description: &str,
) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert("kind".into(), kind.into());
    map.insert("source".into(), source.into());
    if let Some(revision) = revision {
        map.insert("revision".into(), revision.into());
    }
    map.insert("fingerprint".into(), fingerprint.into());
    map.insert("description".into(), description.into());
    serde_json::Value::Object(map)
}

fn file_with(name: &str, entry: serde_json::Value) -> String {
    let mut buckets = serde_json::Map::new();
    buckets.insert(name.to_owned(), entry);
    serde_json::json!({"schema_version": 1, "buckets": buckets}).to_string()
}

fn git_entry() -> serde_json::Value {
    entry(
        "git",
        "https://github.com/owner/bucket",
        Some("main"),
        "0e250ad52a3ba2aa",
        "A community bucket.",
    )
}

fn error_of(json: &str) -> String {
    parse_file(json).expect_err("refusal").to_string()
}

#[test]
fn known_buckets_parse_accepts_valid_entries_and_sorts_by_name() {
    let known = parse_file(VALID).expect("valid file");
    assert_eq!(known.len(), 2);
    assert!(!known.is_empty());
    assert_eq!(known.entries()[0].name, "alpha");
    assert_eq!(known.entries()[0].kind.as_str(), "git");
    assert_eq!(known.entries()[0].revision.as_deref(), Some("main"));
    assert_eq!(known.entries()[1].name, "zeta");
    assert_eq!(known.entries()[1].kind.as_str(), "zip");
    assert_eq!(known.entries()[1].revision, None);
    assert_eq!(
        known.entries()[1].fingerprint.to_string(),
        "ABCDEF0123456789",
        "lowercase input is stored in the canonical uppercase form"
    );
    assert_eq!(
        known.get("alpha").map(|held| held.source.as_str()),
        Some("https://github.com/owner/alpha-bucket")
    );
    assert!(known.get("missing").is_none());
}

#[test]
fn known_buckets_refuse_a_wrong_schema_version() {
    let error = error_of(r#"{"schema_version":2,"buckets":{}}"#);
    assert!(error.contains("schema_version 2"), "{error}");
}

#[test]
fn known_buckets_refuse_unknown_fields_at_every_level() {
    let error = error_of(r#"{"schema_version":1,"buckets":{},"extra":true}"#);
    assert!(error.contains("unknown field"), "{error}");
    assert!(error.contains("extra"), "{error}");

    let mut raw = git_entry();
    raw["extra"] = serde_json::json!("surprise");
    let error = error_of(&file_with("foo", raw));
    assert!(error.contains("known bucket \"foo\""), "{error}");
    assert!(error.contains("extra"), "{error}");
}

#[test]
fn known_buckets_refuse_an_unknown_kind() {
    let error = error_of(&file_with(
        "foo",
        entry(
            "hg",
            "https://example.com/bucket",
            None,
            "0e250ad52a3ba2aa",
            "Bucket.",
        ),
    ));
    assert!(error.contains("known bucket \"foo\""), "{error}");
    assert!(error.contains("kind"), "{error}");
}

#[test]
fn known_buckets_refuse_non_https_sources() {
    let sources = [
        "http://example.com/bucket",
        "file:///c:/bucket",
        "ssh://git@example.com/bucket",
        "git@example.com:owner/bucket.git",
        "C:/work/bucket",
        "/home/user/bucket",
        "https://user:pass@example.com/bucket",
        "https://",
        "https:///owner/bucket",
        "https://exa mple.com/bucket",
        "https://example.com/bucket\tname",
        "https:\\example.com\\bucket",
    ];
    for source in sources {
        let error = error_of(&file_with(
            "foo",
            entry("git", source, Some("main"), "0e250ad52a3ba2aa", "Bucket."),
        ));
        assert!(error.contains("known bucket \"foo\""), "{source}: {error}");
        assert!(error.contains("source"), "{source}: {error}");
    }
}

#[test]
fn known_buckets_refuse_an_oversized_source() {
    let source = format!("https://example.com/{}", "a".repeat(2048));
    let error = error_of(&file_with(
        "foo",
        entry("git", &source, Some("main"), "0e250ad52a3ba2aa", "Bucket."),
    ));
    assert!(error.contains("known bucket \"foo\""), "{error}");
    assert!(error.contains("source"), "{error}");
}

#[test]
fn known_buckets_refuse_bad_git_revisions() {
    let long = "a".repeat(256);
    for revision in ["", long.as_str(), "-main", "feature branch", "main\tbranch"] {
        let error = error_of(&file_with(
            "foo",
            entry(
                "git",
                "https://example.com/bucket",
                Some(revision),
                "0e250ad52a3ba2aa",
                "Bucket.",
            ),
        ));
        assert!(
            error.contains("known bucket \"foo\""),
            "{revision}: {error}"
        );
        assert!(error.contains("revision"), "{revision}: {error}");
    }
}

#[test]
fn known_buckets_refuse_a_revision_on_a_zip_entry() {
    let error = error_of(&file_with(
        "foo",
        entry(
            "zip",
            "https://example.com/bucket.zip",
            Some("main"),
            "0e250ad52a3ba2aa",
            "Bucket.",
        ),
    ));
    assert!(error.contains("known bucket \"foo\""), "{error}");
    assert!(error.contains("revision"), "{error}");
}

#[test]
fn known_buckets_refuse_invalid_fingerprints() {
    for fingerprint in [
        "",
        "0e250ad5",
        "0e250ad52a3ba2aaa",
        "0e250ad52a3ba2ag",
        "zzzzzzzzzzzzzzzz",
    ] {
        let error = error_of(&file_with(
            "foo",
            entry(
                "git",
                "https://example.com/bucket",
                Some("main"),
                fingerprint,
                "Bucket.",
            ),
        ));
        assert!(
            error.contains("known bucket \"foo\""),
            "{fingerprint}: {error}"
        );
        assert!(error.contains("fingerprint"), "{fingerprint}: {error}");
    }
}

#[test]
fn known_buckets_refuse_invalid_and_reserved_names() {
    let long = "a".repeat(65);
    for name in ["main", "Community", "1bucket", "bucket-", long.as_str(), ""] {
        let error = error_of(&file_with(name, git_entry()));
        assert!(error.contains("invalid name"), "{name}: {error}");
        assert!(
            error.contains("not the reserved name main"),
            "{name}: {error}"
        );
    }
    let error = error_of(&file_with("main", git_entry()));
    assert!(error.contains("known bucket \"main\""), "{error}");
}

#[test]
fn known_buckets_refuse_bad_descriptions() {
    let long = "a".repeat(201);
    for description in ["", long.as_str(), "line\nbreak", "tab\there"] {
        let error = error_of(&file_with(
            "foo",
            entry(
                "git",
                "https://example.com/bucket",
                Some("main"),
                "0e250ad52a3ba2aa",
                description,
            ),
        ));
        assert!(error.contains("known bucket \"foo\""), "{error}");
        assert!(error.contains("description"), "{error}");
    }
}

#[test]
fn known_buckets_refuse_duplicate_names() {
    let error = error_of(
        r#"{"schema_version":1,"buckets":{
            "foo":{"kind":"git","source":"https://example.com/a","revision":"main","fingerprint":"0e250ad52a3ba2aa","description":"First."},
            "foo":{"kind":"git","source":"https://example.com/b","revision":"main","fingerprint":"0e250ad52a3ba2aa","description":"Second."}
        }}"#,
    );
    assert!(error.contains("known bucket \"foo\""), "{error}");
    assert!(error.contains("listed twice"), "{error}");
}

#[test]
fn known_buckets_refuse_more_than_256_entries() {
    let mut buckets = serde_json::Map::new();
    for index in 0..257 {
        buckets.insert(format!("bucket-{index}"), git_entry());
    }
    let json = serde_json::json!({"schema_version": 1, "buckets": buckets}).to_string();
    let error = error_of(&json);
    assert!(error.contains("limit is 256"), "{error}");
}

#[test]
fn known_buckets_refuse_an_oversized_file() {
    let mut bytes = VALID.as_bytes().to_vec();
    bytes.resize(256 * 1024 + 1, b' ');
    let error = KnownBuckets::parse(&bytes)
        .expect_err("oversized file")
        .to_string();
    assert!(error.contains("size limit"), "{error}");
}

fn seed_main(root: &Path, known: Option<&str>) {
    let main = root.join("buckets/main");
    fs::create_dir_all(&main).expect("fixture main");
    fs::write(
        main.join("bucket.json"),
        r#"{"schema_version":2,"name":"main","title":"Main"}"#,
    )
    .expect("fixture metadata");
    if let Some(known) = known {
        fs::write(main.join(KNOWN_BUCKETS_FILE), known).expect("fixture known buckets");
    }
}

#[test]
fn known_buckets_loader_is_empty_when_the_file_is_absent() {
    let data = tempfile::tempdir().expect("fixture");
    seed_main(data.path(), None);
    let store = BucketStore::new(&ManagerRoots::new(data.path(), None));
    let known = store.known_buckets().expect("loader");
    assert!(known.is_empty());
    assert_eq!(known.len(), 0);
}

#[test]
fn known_buckets_loader_reads_the_official_main_copy() {
    let data = tempfile::tempdir().expect("fixture");
    seed_main(data.path(), Some(VALID));
    let store = BucketStore::new(&ManagerRoots::new(data.path(), None));
    let known = store.known_buckets().expect("loader");
    assert_eq!(known.len(), 2);
    assert_eq!(known.entries()[0].name, "alpha");
}

#[test]
fn known_buckets_loader_reads_the_state_dir_copy() {
    let data = tempfile::tempdir().expect("fixture");
    let profile = tempfile::tempdir().expect("fixture profile");
    seed_main(profile.path(), Some(VALID));
    let store = BucketStore::new(&ManagerRoots::new(data.path(), Some(profile.path())));
    let known = store.known_buckets().expect("loader");
    assert_eq!(known.len(), 2);
    assert!(
        !data.path().join("buckets").exists(),
        "shared data directory stays untouched"
    );
}

#[test]
fn known_buckets_loader_reports_a_malformed_file() {
    let data = tempfile::tempdir().expect("fixture");
    let malformed = file_with(
        "foo",
        entry(
            "git",
            "https://example.com/bucket",
            Some("main"),
            "not-a-key",
            "Bucket.",
        ),
    );
    seed_main(data.path(), Some(&malformed));
    let store = BucketStore::new(&ManagerRoots::new(data.path(), None));
    let error = store
        .known_buckets()
        .expect_err("malformed file")
        .to_string();
    assert!(error.contains("known bucket \"foo\""), "{error}");
    assert!(error.contains("fingerprint"), "{error}");
}

fn pack(tree: &Path, zip: &Path) {
    fn files(current: &Path, root: &Path, output: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(current).expect("fixture entries") {
            let path = entry.expect("fixture entry").path();
            if path.is_dir() {
                files(&path, root, output);
            } else {
                output.push((
                    path.strip_prefix(root)
                        .expect("fixture relative path")
                        .to_str()
                        .expect("fixture UTF-8 path")
                        .replace('\\', "/"),
                    fs::read(&path).expect("fixture file"),
                ));
            }
        }
    }
    let mut entries = Vec::new();
    files(tree, tree, &mut entries);
    entries.sort();
    let mut writer = zip::ZipWriter::new(fs::File::create(zip).expect("fixture ZIP"));
    for (name, bytes) in entries {
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .expect("fixture ZIP entry");
        writer.write_all(&bytes).expect("fixture ZIP bytes");
    }
    writer.finish().expect("fixture ZIP complete");
}

fn install(store: &BucketStore, tree: &Path, parent: &Path) -> anyhow::Result<()> {
    let zip = parent.join("main.zip");
    pack(tree, &zip);
    let key = KeyPair::generate_unencrypted_keypair().expect("fixture key");
    let public = key.pk.to_box().expect("fixture key box").into_string();
    let statement = crate::buckets::catalog_statement(tree).expect("fixture statement");
    let signature = minisign::sign(
        Some(&key.pk),
        &key.sk,
        Cursor::new(statement.as_bytes()),
        Some("known buckets fixture"),
        Some("known buckets fixture signature"),
    )
    .expect("fixture signature")
    .into_string();
    store
        .update_main(
            &public,
            || -> anyhow::Result<fs::File> { Ok(fs::File::open(&zip)?) },
            || -> anyhow::Result<io::Cursor<Vec<u8>>> {
                Ok(io::Cursor::new(signature.clone().into_bytes()))
            },
        )
        .map(|_| ())
}

fn tree_with(parent: &Path, known: Option<&str>) -> PathBuf {
    let tree = parent.join("tree");
    fs::create_dir_all(&tree).expect("fixture tree");
    fs::write(
        tree.join("bucket.json"),
        r#"{"schema_version":2,"name":"main","title":"Main"}"#,
    )
    .expect("fixture metadata");
    if let Some(known) = known {
        fs::write(tree.join(KNOWN_BUCKETS_FILE), known).expect("fixture known buckets");
    }
    tree
}

#[test]
fn known_buckets_main_update_accepts_a_root_file() {
    let data = tempfile::tempdir().expect("fixture");
    let tree = tree_with(data.path(), Some(VALID));
    let store = BucketStore::new(&ManagerRoots::new(data.path(), None));
    install(&store, &tree, data.path()).expect("main update");
    let installed = data.path().join("buckets/main").join(KNOWN_BUCKETS_FILE);
    assert_eq!(
        fs::read_to_string(installed).expect("installed file"),
        VALID
    );
    assert_eq!(store.known_buckets().expect("loader").len(), 2);
}

#[test]
fn known_buckets_main_update_refuses_a_malformed_file_and_keeps_the_installed_copy() {
    let data = tempfile::tempdir().expect("fixture");
    let store = BucketStore::new(&ManagerRoots::new(data.path(), None));
    let valid = tree_with(&data.path().join("first"), Some(VALID));
    fs::create_dir_all(data.path().join("first")).expect("fixture parent");
    install(&store, &valid, &data.path().join("first")).expect("first main update");
    let installed = data.path().join("buckets/main");
    let before = crate::bucket_digest::directory_digest(&installed).expect("installed digest");

    let malformed = file_with(
        "foo",
        entry(
            "git",
            "https://example.com/bucket",
            Some("main"),
            "too-short",
            "Bucket.",
        ),
    );
    let second = data.path().join("second");
    fs::create_dir_all(&second).expect("fixture parent");
    let tree = tree_with(&second, Some(&malformed));
    let error = install(&store, &tree, &second)
        .expect_err("malformed known buckets")
        .to_string();
    assert!(error.contains("known bucket \"foo\""), "{error}");
    assert!(error.contains("fingerprint"), "{error}");
    assert_eq!(
        crate::bucket_digest::directory_digest(&installed).expect("installed digest"),
        before,
        "installed copy is unchanged"
    );
}

#[test]
fn known_buckets_other_bucket_file_is_never_parsed() {
    let data = tempfile::tempdir().expect("fixture");
    seed_main(data.path(), Some(VALID));
    let store = BucketStore::new(&ManagerRoots::new(data.path(), None));
    let bucket = tempfile::tempdir().expect("fixture bucket");
    fs::write(
        bucket.path().join("bucket.json"),
        r#"{"schema_version":2,"name":"community","title":"Community"}"#,
    )
    .expect("fixture metadata");
    fs::write(bucket.path().join(KNOWN_BUCKETS_FILE), "{not json").expect("fixture file");
    store
        .add("community", bucket.path())
        .expect("registration ignores the root file");
    let known = store.known_buckets().expect("loader");
    assert_eq!(known.len(), 2, "only the main copy is read");
    assert_eq!(known.entries()[0].name, "alpha");
}

#[test]
fn known_buckets_fingerprints_compare_with_the_catalog_key_id() {
    let key = CatalogKeyId::parse_hex("0123456789abcdef").expect("fixture key ID");
    let known = parse_file(VALID).expect("valid file");
    assert_eq!(known.get("alpha").map(|held| held.fingerprint), Some(key));
}
