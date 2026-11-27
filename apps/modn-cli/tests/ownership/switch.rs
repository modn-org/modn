//! Profile switch, its dry run and the extra profile's install for the ownership model.

use super::snapshot::{files, json, path_key, snapshot, stderr, Snapshot};
use super::{check_install, lua_string, payload, source, Case, Observed, World};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Output;
use zip::write::SimpleFileOptions;

const SECOND: &str = "second";

/// Uninstall `id`, expecting exactly `owned` to leave and `restored` to replace bytes.
pub(crate) fn check_uninstall(
    world: &World,
    id: &str,
    owned: &BTreeSet<PathBuf>,
    restored: &BTreeMap<String, Vec<u8>>,
) -> Result<(), TestCaseError> {
    let before = snapshot(&world.root)?;
    let removed = world.uninstall(id)?;
    prop_assert!(
        removed.status.success(),
        "uninstall of {} failed: {}",
        id,
        stderr(&removed)
    );
    let after = snapshot(&world.root)?;
    let owned: BTreeSet<String> = owned.iter().map(|path| path_key(path)).collect();
    let mut expected: BTreeMap<PathBuf, Vec<u8>> = files(&before)
        .into_iter()
        .filter(|(path, _)| !owned.contains(&path_key(path)))
        .map(|(path, bytes)| (path.clone(), bytes.clone()))
        .collect();
    for (path, bytes) in &mut expected {
        if let Some(replacement) = restored.get(&path_key(path)) {
            *bytes = replacement.clone();
        }
    }
    let after_files: BTreeMap<PathBuf, Vec<u8>> = files(&after)
        .into_iter()
        .map(|(path, bytes)| (path.clone(), bytes.clone()))
        .collect();
    prop_assert_eq!(
        after_files,
        expected,
        "uninstall of {} removed the wrong files",
        id
    );
    Ok(())
}

fn tree(snapshot: &Snapshot) -> BTreeMap<String, Vec<u8>> {
    files(snapshot)
        .into_iter()
        .map(|(path, bytes)| (path_key(path), bytes.clone()))
        .collect()
}

fn key(root: &str, path: &str) -> String {
    path_key(&Path::new(root).join(path))
}

fn plan_paths(plan: &serde_json::Value, field: &str) -> Result<BTreeSet<String>, TestCaseError> {
    plan[field]
        .as_array()
        .ok_or_else(|| TestCaseError::fail(format!("switch plan has no {field} list")))?
        .iter()
        .map(|entry| {
            let root = entry["root"]
                .as_str()
                .ok_or_else(|| TestCaseError::fail("switch plan path has no root"))?;
            let path = entry["path"]
                .as_str()
                .ok_or_else(|| TestCaseError::fail("switch plan path has no path"))?;
            Ok(key(root, path))
        })
        .collect()
}

fn changed(before: &Snapshot, after: &Snapshot) -> BTreeSet<String> {
    let before = tree(before);
    let after = tree(after);
    before
        .keys()
        .chain(after.keys())
        .filter(|path| before.get(*path) != after.get(*path))
        .cloned()
        .collect()
}

impl World {
    fn create_profile(&self, name: &str) -> Result<Output, TestCaseError> {
        let game = self.game.to_string_lossy().into_owned();
        self.modn(&["profile", "create", name, "--path", &game])
    }

    fn switch(&self, name: &str, dry_run: bool) -> Result<Output, TestCaseError> {
        let game = self.game.to_string_lossy().into_owned();
        let mut args = vec!["profile", "switch", name, "--path", &game];
        if dry_run {
            args.push("--dry-run");
        } else {
            args.push("--yes");
        }
        self.modn(&args)
    }

    /// Add a third package whose members are `destinations` to the loaded module and scratch folder.
    fn add_extra_package(&self, destinations: &[String]) -> Result<(), TestCaseError> {
        let module = self.modules.join("cyberpunk-2077/module.lua");
        let mut code = fs::read_to_string(&module)?;
        let marker = "local mapping = {\n";
        let offset = code
            .find(marker)
            .ok_or_else(|| TestCaseError::fail("module has no mapping table"))?
            + marker.len();
        let mut entries = String::new();
        for (index, destination) in destinations.iter().enumerate() {
            entries.push_str(&format!(
                "  {{ {}, {} }},\n",
                lua_string(&source('c', index)),
                lua_string(destination)
            ));
        }
        code.insert_str(offset, &entries);
        fs::write(&module, code)?;
        let mut zip = zip::ZipWriter::new(File::create(self.archive('c'))?);
        for index in 0..destinations.len() {
            zip.start_file(source('c', index), SimpleFileOptions::default())
                .map_err(|err| TestCaseError::fail(err.to_string()))?;
            zip.write_all(&payload('c', index))?;
        }
        zip.finish()
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        Ok(())
    }
}

/// Report the overlapping claims before the switch changes which profile is active.
fn check_conflicts(
    world: &World,
    first_id: &str,
    second_id: &str,
    first_owned: &BTreeSet<PathBuf>,
    second_owned: &BTreeSet<PathBuf>,
) -> Result<(), TestCaseError> {
    let before = snapshot(&world.root)?;
    let state_before = (world.state.exists(), snapshot(&world.state)?);
    let game = world.game.to_string_lossy().into_owned();
    let output = world.modn(&["conflicts", "--path", &game])?;
    prop_assert!(
        output.status.success(),
        "conflicts failed: {}",
        stderr(&output)
    );
    prop_assert_eq!(
        &snapshot(&world.root)?,
        &before,
        "conflicts changed the game"
    );
    prop_assert_eq!(
        &(world.state.exists(), snapshot(&world.state)?),
        &state_before,
        "conflicts changed manager state"
    );
    let report = json(&output)?;
    prop_assert_eq!(
        report["active"].as_bool(),
        Some(true),
        "conflicts did not describe the active profile"
    );
    let entries = report["conflicts"]
        .as_array()
        .ok_or_else(|| TestCaseError::fail("conflicts report has no conflicts list"))?;
    let second_keys: BTreeSet<String> = second_owned.iter().map(|path| path_key(path)).collect();
    let expected: BTreeSet<String> = first_owned
        .iter()
        .map(|path| path_key(path))
        .filter(|path| second_keys.contains(path))
        .collect();
    let actual: BTreeSet<String> = entries
        .iter()
        .map(|entry| {
            let root = entry["root"]
                .as_str()
                .ok_or_else(|| TestCaseError::fail("conflict has no root"))?;
            let path = entry["path"]
                .as_str()
                .ok_or_else(|| TestCaseError::fail("conflict has no path"))?;
            Ok(key(root, path))
        })
        .collect::<Result<_, TestCaseError>>()?;
    prop_assert_eq!(
        &actual,
        &expected,
        "conflicts named the wrong overlapping paths"
    );
    for entry in entries {
        prop_assert_eq!(
            entry["winner"].as_str(),
            Some(second_id),
            "conflicts named the wrong winner"
        );
        prop_assert_eq!(
            entry["consistent"].as_bool(),
            Some(true),
            "conflicts reported an inconsistent winner"
        );
        let packages: BTreeSet<&str> = entry["claims"]
            .as_array()
            .map(|claims| {
                claims
                    .iter()
                    .filter_map(|claim| claim["package"].as_str())
                    .collect()
            })
            .unwrap_or_default();
        prop_assert_eq!(
            packages,
            BTreeSet::from([first_id, second_id]),
            "conflicts did not name both claimants"
        );
    }
    Ok(())
}

/// Switch to an empty second profile, install one extra package there, and switch back.
pub(crate) fn check_switch_round_trip(
    world: &World,
    case: &Case,
    first_id: &str,
    second_id: &str,
    first_owned: &BTreeSet<PathBuf>,
    second_owned: &BTreeSet<PathBuf>,
    observed: &mut Observed,
) -> Result<Option<String>, TestCaseError> {
    check_conflicts(world, first_id, second_id, first_owned, second_owned)?;
    let before = snapshot(&world.root)?;
    let created = world.create_profile(SECOND)?;
    prop_assert!(
        created.status.success(),
        "profile create failed: {}",
        stderr(&created)
    );
    prop_assert_eq!(
        &snapshot(&world.root)?,
        &before,
        "profile create changed the game"
    );
    let state_before = (world.state.exists(), snapshot(&world.state)?);
    let dry = world.switch(SECOND, true)?;
    prop_assert!(
        dry.status.success(),
        "dry run of switching to the second profile failed: {}",
        stderr(&dry)
    );
    prop_assert_eq!(
        &snapshot(&world.root)?,
        &before,
        "dry run of switching to the second profile changed the game"
    );
    prop_assert_eq!(
        &(world.state.exists(), snapshot(&world.state)?),
        &state_before,
        "dry run of switching to the second profile changed manager state"
    );
    let plan = json(&dry)?;
    let active = plan["from"]
        .as_str()
        .ok_or_else(|| TestCaseError::fail("switch plan names no active profile"))?
        .to_owned();
    let removes = plan_paths(&plan, "remove")?;
    let writes = plan_paths(&plan, "write")?;
    let owned: BTreeSet<String> = first_owned
        .iter()
        .chain(second_owned)
        .map(|path| path_key(path))
        .collect();
    prop_assert_eq!(&removes, &owned, "switch preview named the wrong removals");
    prop_assert!(
        writes.is_empty(),
        "switch to an empty profile previewed writes: {writes:?}"
    );
    let switched = world.switch(SECOND, false)?;
    prop_assert!(
        switched.status.success(),
        "switching to the second profile failed: {}",
        stderr(&switched)
    );
    observed.switches += 1;
    let mid = snapshot(&world.root)?;
    let expected: BTreeMap<String, Vec<u8>> = tree(&before)
        .into_iter()
        .filter(|(path, _)| !owned.contains(path))
        .collect();
    prop_assert_eq!(
        tree(&mid),
        expected,
        "switching to the second profile changed an unmanaged file"
    );

    world.add_extra_package(&case.first)?;
    let extra = check_install(world, 'c', case.first.len(), observed)?;
    let Some((extra_id, _added, _extra_owned)) = extra else {
        switch_back(world, &active, &before)?;
        return Ok(None);
    };

    switch_back(world, &active, &before)?;
    if case.user_files.len().is_multiple_of(2) {
        check_uninstall(world, &extra_id, &BTreeSet::new(), &BTreeMap::new())?;
        prop_assert_eq!(
            &snapshot(&world.root)?,
            &before,
            "uninstalling the extra package changed the game"
        );
        return Ok(None);
    }
    Ok(Some(extra_id))
}

/// Dry run and then perform the switch back, checking that the dry run wrote nothing.
fn switch_back(world: &World, active: &str, before: &Snapshot) -> Result<(), TestCaseError> {
    let mid = snapshot(&world.root)?;
    let state_before = (world.state.exists(), snapshot(&world.state)?);
    let dry = world.switch(active, true)?;
    prop_assert!(
        dry.status.success(),
        "dry run of switching back to the active profile failed: {}",
        stderr(&dry)
    );
    prop_assert_eq!(
        &snapshot(&world.root)?,
        &mid,
        "dry run of switching back to the active profile changed the game"
    );
    prop_assert_eq!(
        &(world.state.exists(), snapshot(&world.state)?),
        &state_before,
        "dry run of switching back to the active profile changed manager state"
    );
    let plan = json(&dry)?;
    let switched = world.switch(active, false)?;
    prop_assert!(
        switched.status.success(),
        "switching back to the active profile failed: {}",
        stderr(&switched)
    );
    let after = snapshot(&world.root)?;
    prop_assert_eq!(
        &after,
        before,
        "switching back to the active profile did not restore the game"
    );
    let removes = plan_paths(&plan, "remove")?;
    let writes = plan_paths(&plan, "write")?;
    let previewed: BTreeSet<String> = removes.union(&writes).cloned().collect();
    prop_assert_eq!(
        changed(&mid, &after),
        previewed,
        "switching back to the active profile changed paths the preview did not name"
    );
    Ok(())
}

/// Uninstall a package left in the inactive profile after the active packages are gone.
pub(crate) fn finish(
    world: &World,
    extra: Option<String>,
    start: &Snapshot,
) -> Result<(), TestCaseError> {
    let Some(id) = extra else {
        return Ok(());
    };
    check_uninstall(world, &id, &BTreeSet::new(), &BTreeMap::new())?;
    prop_assert_eq!(
        &snapshot(&world.root)?,
        start,
        "uninstalling the extra package did not restore the initial game"
    );
    Ok(())
}
