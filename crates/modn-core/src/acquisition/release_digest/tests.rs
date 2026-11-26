use super::{ArchiveDigest, ReleaseDigest};

const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn a_release_digest_refuses_an_empty_string() {
    assert!(ReleaseDigest::pinned("").is_err());
    assert!(ReleaseDigest::computed("").is_err());
    assert!(ReleaseDigest::from_pin(Some("")).is_err());
}

#[test]
fn a_release_digest_refuses_a_non_digest_string() {
    for value in [
        "0".repeat(63),
        "0".repeat(65),
        "A".repeat(64),
        "g".repeat(64),
        format!("{}x", &DIGEST[..63]),
    ] {
        assert!(ReleaseDigest::pinned(&value).is_err(), "{value}");
        assert!(ReleaseDigest::computed(&value).is_err(), "{value}");
    }
}

#[test]
fn a_missing_catalog_pin_is_unknown() {
    assert_eq!(
        ReleaseDigest::from_pin(None).expect("missing pin"),
        ReleaseDigest::Unknown
    );
}

#[test]
fn a_known_digest_keeps_its_source_and_value() {
    let pinned = ReleaseDigest::pinned(DIGEST).expect("pin");
    let computed = ReleaseDigest::computed(DIGEST).expect("computed");
    assert!(matches!(pinned, ReleaseDigest::Pinned(_)));
    assert!(matches!(computed, ReleaseDigest::Computed(_)));
    assert_eq!(pinned.as_str(), Some(DIGEST));
    assert_eq!(computed.as_str(), Some(DIGEST));
    assert_eq!(pinned, DIGEST);
}

#[test]
fn stored_declarations_use_the_hex_text_or_an_empty_unknown() {
    let pinned = ReleaseDigest::pinned(DIGEST).expect("pin");
    let json = serde_json::to_string(&pinned).expect("serialize");
    assert_eq!(json, format!("\"{DIGEST}\""));

    let decoded: ReleaseDigest = serde_json::from_str(&json).expect("decode");
    assert_eq!(decoded, pinned);

    let unknown: ReleaseDigest = serde_json::from_str("\"\"").expect("decode unknown");
    assert_eq!(unknown, ReleaseDigest::Unknown);
    assert_eq!(
        serde_json::to_string(&unknown).expect("serialize unknown"),
        "\"\""
    );
}

#[test]
fn an_archive_digest_displays_its_hex_text() {
    let digest = ArchiveDigest::new(DIGEST).expect("digest");
    assert_eq!(digest.to_string(), DIGEST);
}
