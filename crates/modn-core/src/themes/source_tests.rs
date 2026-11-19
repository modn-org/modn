use crate::themes::{ThemeSource, ThemeSourceKind};
use serde_json::json;

#[test]
fn official_source_serializes_documented_shape() {
    let value = serde_json::to_value(ThemeSource::official()).expect("serialize");
    assert_eq!(
        value,
        json!({"kind":"official","bucket":"main","label":"main bucket"})
    );
}

#[test]
fn bucket_source_serializes_documented_shape() {
    let value = serde_json::to_value(ThemeSource::bucket("community")).expect("serialize");
    assert_eq!(
        value,
        json!({"kind":"bucket","bucket":"community","label":"community"})
    );
}

#[test]
fn local_source_serializes_documented_shape() {
    let value = serde_json::to_value(ThemeSource::local()).expect("serialize");
    assert_eq!(
        value,
        json!({"kind":"local","bucket":null,"label":"local registry"})
    );
}

#[test]
fn source_kinds_serialize_lowercase() {
    assert_eq!(
        serde_json::to_value(ThemeSourceKind::Official).expect("serialize"),
        json!("official")
    );
    assert_eq!(
        serde_json::to_value(ThemeSourceKind::Bucket).expect("serialize"),
        json!("bucket")
    );
    assert_eq!(
        serde_json::to_value(ThemeSourceKind::Local).expect("serialize"),
        json!("local")
    );
}

#[test]
fn source_accessors_return_the_parts() {
    let official = ThemeSource::official();
    assert_eq!(official.kind(), ThemeSourceKind::Official);
    assert_eq!(official.bucket_id(), Some("main"));
    assert_eq!(official.label(), "main bucket");

    let bucket = ThemeSource::bucket("community");
    assert_eq!(bucket.kind(), ThemeSourceKind::Bucket);
    assert_eq!(bucket.bucket_id(), Some("community"));

    let local = ThemeSource::local();
    assert_eq!(local.kind(), ThemeSourceKind::Local);
    assert_eq!(local.bucket_id(), None);
    assert_eq!(local.label(), "local registry");
}

#[test]
fn source_precedence_is_official_then_local_then_bucket() {
    assert!(
        ThemeSourceKind::Official.precedence() < ThemeSourceKind::Local.precedence(),
        "official must win over local"
    );
    assert!(
        ThemeSourceKind::Local.precedence() < ThemeSourceKind::Bucket.precedence(),
        "local must win over a bucket"
    );
}
