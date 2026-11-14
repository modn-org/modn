//! The fixture dry-run plan carries the archive digest the desktop rechecks.

use super::support::Fixture;
use sha2::{Digest, Sha256};
use std::fs;

#[test]
fn the_dry_run_plan_carries_the_archive_sha256() {
    let f = Fixture::new();
    let review = f.backend.review(&f.game, &f.source).expect("review");
    let archive = fs::read(&review.archive).expect("archive bytes");
    let expected = format!("{:x}", Sha256::digest(&archive));
    assert_eq!(
        review.plan["archive_sha256"].as_str(),
        Some(expected.as_str()),
        "the dry-run plan must return the archive SHA-256 the desktop rechecks"
    );
}
