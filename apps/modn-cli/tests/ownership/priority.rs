//! Profile priority reorder and apply round trip for the ownership model.

use super::snapshot::{files, path_key, snapshot, stderr, Snapshot};
use super::{payload, World};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use std::collections::BTreeMap;
use std::path::Path;

fn tree(snapshot: &Snapshot) -> BTreeMap<String, Vec<u8>> {
    files(snapshot)
        .into_iter()
        .map(|(path, bytes)| (path_key(path), bytes.clone()))
        .collect()
}

pub(crate) fn check_priority_round_trip(
    world: &World,
    first_id: &str,
    second_id: &str,
    first: &[String],
) -> Result<(), TestCaseError> {
    let before = snapshot(&world.root)?;
    let moved = world.priority(first_id, 1, true)?;
    prop_assert!(
        moved.status.success(),
        "priority apply of {} failed: {}",
        first_id,
        stderr(&moved)
    );
    let mut expected = tree(&before);
    for (index, destination) in first.iter().enumerate() {
        let key = path_key(&Path::new("game").join(destination));
        expected.insert(key, payload('a', index));
    }
    let after = snapshot(&world.root)?;
    prop_assert_eq!(
        tree(&after),
        expected,
        "priority apply of {} left the wrong files",
        first_id
    );
    let restored = world.priority(second_id, 1, true)?;
    prop_assert!(
        restored.status.success(),
        "priority apply of {} failed: {}",
        second_id,
        stderr(&restored)
    );
    prop_assert_eq!(
        snapshot(&world.root)?,
        before,
        "restoring the priority order did not restore the files"
    );
    Ok(())
}
