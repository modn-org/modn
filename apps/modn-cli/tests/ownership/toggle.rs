//! Profile disable and enable round trip for the ownership model.

use super::snapshot::{files, path_key, snapshot, stderr};
use super::{payload, World};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub(crate) fn check_toggle_round_trip(
    world: &World,
    id: &str,
    owned: &BTreeSet<PathBuf>,
    first: &[String],
) -> Result<(), TestCaseError> {
    let before = snapshot(&world.root)?;
    let disabled = world.toggle(id, false)?;
    prop_assert!(
        disabled.status.success(),
        "disable of {} failed: {}",
        id,
        stderr(&disabled)
    );
    let first_payloads: BTreeMap<String, Vec<u8>> = first
        .iter()
        .enumerate()
        .map(|(index, destination)| {
            (
                path_key(&Path::new("game").join(destination)),
                payload('a', index),
            )
        })
        .collect();
    let mut expected: BTreeMap<String, Vec<u8>> = files(&before)
        .into_iter()
        .map(|(path, bytes)| (path_key(path), bytes.clone()))
        .collect();
    for path in owned {
        let key = path_key(path);
        if let Some(bytes) = first_payloads.get(&key) {
            expected.insert(key, bytes.clone());
        } else {
            expected.remove(&key);
        }
    }
    let after: BTreeMap<String, Vec<u8>> = files(&snapshot(&world.root)?)
        .into_iter()
        .map(|(path, bytes)| (path_key(path), bytes.clone()))
        .collect();
    prop_assert_eq!(after, expected, "disable of {} left the wrong files", id);
    let enabled = world.toggle(id, true)?;
    prop_assert!(
        enabled.status.success(),
        "enable of {} failed: {}",
        id,
        stderr(&enabled)
    );
    prop_assert_eq!(
        snapshot(&world.root)?,
        before,
        "enable of {} did not restore the winner's files",
        id
    );
    Ok(())
}
