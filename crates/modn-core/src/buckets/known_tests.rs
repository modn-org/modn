use super::test_support::{entry, file_with, git_entry, VALID};
use super::KnownBuckets;
use crate::buckets::CatalogKeyId;

fn parse_file(text: &str) -> anyhow::Result<KnownBuckets> {
    KnownBuckets::parse(text.as_bytes())
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

#[test]
fn known_buckets_fingerprints_compare_with_the_catalog_key_id() {
    let key = CatalogKeyId::parse_hex("0123456789abcdef").expect("fixture key ID");
    let known = parse_file(VALID).expect("valid file");
    assert_eq!(known.get("alpha").map(|held| held.fingerprint), Some(key));
}
