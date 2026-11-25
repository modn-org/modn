use super::support::Fixture;
use std::fs;

#[test]
fn the_dry_run_plan_carries_the_archive_blake3() {
    let f = Fixture::new();
    let review = f.backend.review(&f.game, &f.source).expect("review");
    let archive = fs::read(&review.archive).expect("archive bytes");
    let expected = blake3::hash(&archive).to_hex().to_string();
    assert_eq!(
        review.plan["archive_blake3"].as_str(),
        Some(expected.as_str()),
        "the dry-run plan must return the archive BLAKE3 digest the desktop rechecks"
    );
}
