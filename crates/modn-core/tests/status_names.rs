//! Typed statuses keep the exact strings used by JSON output and Lua policies.
use modn_core::archive::ArchiveFormat;
use modn_core::planning::PlanStatus;
use modn_core::scan::IntegrityStatus;
use serde_json::Value;

fn json_name<T: serde::Serialize>(value: T) -> String {
    match serde_json::to_value(value).expect("serialize") {
        Value::String(name) => name,
        other => panic!("expected a string, got {other}"),
    }
}

#[test]
fn integrity_status_names_match_json_lua_and_display() {
    for (status, name) in [
        (IntegrityStatus::Intact, "intact"),
        (IntegrityStatus::Present, "present"),
        (IntegrityStatus::Modified, "modified"),
        (IntegrityStatus::Missing, "missing"),
        (IntegrityStatus::Unsafe, "unsafe"),
        (IntegrityStatus::Unreadable, "unreadable"),
    ] {
        assert_eq!(json_name(status), name);
        assert_eq!(status.as_str(), name);
        assert_eq!(status.to_string(), name);
    }
    assert!(IntegrityStatus::Intact.is_in_place());
    assert!(IntegrityStatus::Present.is_in_place());
    assert!(!IntegrityStatus::Modified.is_in_place());
}

#[test]
fn plan_status_and_archive_format_names_are_unchanged() {
    for (status, name) in [
        (PlanStatus::Reviewable, "reviewable"),
        (PlanStatus::Blocked, "blocked"),
    ] {
        assert_eq!(json_name(status), name);
        assert_eq!(status.to_string(), name);
    }
    for (format, name, installable) in [
        (ArchiveFormat::Zip, "zip", true),
        (ArchiveFormat::Jar, "jar", false),
        (ArchiveFormat::SevenZ, "7z", true),
    ] {
        assert_eq!(json_name(format), name);
        assert_eq!(format.as_str(), name);
        assert_eq!(format.to_string(), name);
        assert_eq!(format.is_installable(), installable);
    }
}
