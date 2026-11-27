use crate::main_bucket;
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::{tempdir, TempDir};
use zip::write::SimpleFileOptions;

const SHARED: &[&str] = &[
    "archive/pc/mod/a.archive",
    "archive/pc/mod/A.archive",
    "Archive/PC/Mod/b.archive",
    "archive/pc/mod\u{3a3}/x.archive",
    "archive/pc/mod\u{3c3}/x.archive",
    "r6/scripts/one.reds",
    "R6/Scripts/one.reds",
    "r6/scripts/dir/two.reds",
    "r6/scripts/dir",
    "bin/x64/plugins/note.txt",
    "single.lua",
    "Single.LUA",
    "r6",
];
const HOSTILE: &[&str] = &[
    "../outside.archive",
    "archive/../../outside.archive",
    "/outside.archive",
    "C:/outside.archive",
    "archive/pc/mod/CON.archive",
    "archive/pc/mod/a.archive.",
    "archive/pc/mod/a.archive ",
    "archive/pc/mod/a:stream",
    "archive\\pc\\mod\\back.archive",
    "outside.archive",
    "bin/x64/Cyberpunk2077.exe",
    "bin/x64/Cyberpunk2077.exe/inner.archive",
    "r6/scripts/run.cmd",
    "..",
    "CON",
    "nul.lua",
    "COM1.lua",
    "single.lua.",
    "single.lua ",
    "C:single.lua",
    "//server/share/single.lua",
    "\\\\server\\share\\single.lua",
    "run.bat",
];

fn destination() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => prop::sample::select(SHARED).prop_map(String::from),
        1 => prop::sample::select(HOSTILE).prop_map(String::from),
    ]
}

#[derive(Debug, Clone)]
pub(crate) struct Case {
    pub(crate) user_files: Vec<String>,
    pub(crate) first: Vec<String>,
    pub(crate) second: Vec<String>,
}

pub(crate) fn case() -> impl Strategy<Value = Case> {
    (
        prop::collection::vec(prop::sample::select(SHARED).prop_map(String::from), 0..3),
        prop::collection::vec(destination(), 1..4),
        prop::collection::vec(destination(), 0..3),
    )
        .prop_map(|(user_files, first, second)| Case {
            user_files,
            first,
            second,
        })
}

fn lua_string(text: &str) -> String {
    let mut quoted = String::from("\"");
    for ch in text.chars() {
        match ch {
            '"' | '\\' => {
                quoted.push('\\');
                quoted.push(ch);
            }
            ch if ch.is_control() => quoted.push_str(&format!("\\u{{{:x}}}", u32::from(ch))),
            ch => quoted.push(ch),
        }
    }
    quoted.push('"');
    quoted
}

fn source(package: char, index: usize) -> String {
    format!("{package}/{index}.bin")
}

fn payload(package: char, index: usize) -> Vec<u8> {
    format!("{package}{index}-payload").into_bytes()
}

struct World {
    _temp: TempDir,
    root: PathBuf,
    game: PathBuf,
    state: PathBuf,
    modules: PathBuf,
}

impl World {
    fn new(case: &Case) -> Result<Self, TestCaseError> {
        let temp = tempdir()?;
        main_bucket::seed(temp.path());
        let root = temp.path().join("world");
        let game = root.join("game");
        fs::create_dir_all(game.join("bin/x64"))?;
        fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game")?;
        for (index, file) in case.user_files.iter().enumerate() {
            let path = game.join(file);
            if let Some(parent) = path.parent() {
                if fs::create_dir_all(parent).is_err() {
                    continue;
                }
            }
            let _ = fs::write(&path, format!("user{index}"));
        }
        let modules = temp.path().join("modules");
        let module = modules.join("cyberpunk-2077");
        let bundled =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main/games/cyberpunk-2077");
        main_bucket::copy_main_module(&bundled, &module);
        let code = fs::read_to_string(module.join("module.lua"))?.replace("\r\n", "\n");
        let policy = code
            .strip_suffix("return policy\n")
            .ok_or_else(|| TestCaseError::fail("bundled module does not end in `return policy`"))?;
        let mut map = String::new();
        for (package, destinations) in [('a', &case.first), ('b', &case.second)] {
            for (index, destination) in destinations.iter().enumerate() {
                map.push_str(&format!(
                    "  {{ {}, {} }},\n",
                    lua_string(&source(package, index)),
                    lua_string(destination)
                ));
            }
        }
        fs::write(
            module.join("module.lua"),
            format!(
                "{policy}\nlocal mapping = {{\n{map}}}\nfunction policy.plan(files)\n  \
                 local present = {{}}\n  for _, file in ipairs(files) do present[file.source] = true end\n  \
                 local operations = {{}}\n  for _, pair in ipairs(mapping) do\n    \
                 if present[pair[1]] then operations[#operations + 1] = {{ source = pair[1], \
                 destination = pair[2], kind = 'legacy-archive', package_root = '' }} end\n  end\n  \
                 return {{ schema_version = 1, operations = operations, blockers = {{}} }}\nend\nreturn policy\n"
            ),
        )?;
        for (package, destinations) in [('a', &case.first), ('b', &case.second)] {
            let mut zip =
                zip::ZipWriter::new(File::create(temp.path().join(format!("{package}.zip")))?);
            for index in 0..destinations.len() {
                zip.start_file(source(package, index), SimpleFileOptions::default())
                    .map_err(|err| TestCaseError::fail(err.to_string()))?;
                zip.write_all(&payload(package, index))?;
            }
            zip.finish()
                .map_err(|err| TestCaseError::fail(err.to_string()))?;
        }
        let state = temp.path().join("state");
        Ok(Self {
            root,
            game,
            state,
            modules,
            _temp: temp,
        })
    }

    fn archive(&self, package: char) -> PathBuf {
        self.modules
            .parent()
            .expect("modules live in the scratch folder")
            .join(format!("{package}.zip"))
    }

    fn modn(&self, args: &[&str]) -> Result<Output, TestCaseError> {
        Ok(Command::new(main_bucket::modn_binary())
            .env("LOCALAPPDATA", self._temp.path())
            .env("MODN_GAME_MODULES_DIR", &self.modules)
            .args(["--game", "cyberpunk-2077", "--json", "--state-dir"])
            .arg(&self.state)
            .args(args)
            .output()?)
    }

    fn dry_run(&self, package: char) -> Result<Output, TestCaseError> {
        let archive = self.archive(package);
        let game = self.game.to_string_lossy().into_owned();
        self.modn(&[
            "install",
            &archive.to_string_lossy(),
            "--dry-run",
            "--path",
            &game,
        ])
    }

    fn install(&self, package: char) -> Result<Output, TestCaseError> {
        let archive = self.archive(package);
        let game = self.game.to_string_lossy().into_owned();
        self.modn(&[
            "install",
            &archive.to_string_lossy(),
            "--yes",
            "--path",
            &game,
        ])
    }

    fn uninstall(&self, id: &str) -> Result<Output, TestCaseError> {
        let game = self.game.to_string_lossy().into_owned();
        self.modn(&["uninstall", id, "--yes", "--path", &game])
    }
}

type Snapshot = BTreeMap<PathBuf, Option<Vec<u8>>>;

fn snapshot(root: &Path) -> Result<Snapshot, TestCaseError> {
    let mut entries = BTreeMap::new();
    if fs::symlink_metadata(root).is_err() {
        return Ok(entries);
    }
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)? {
            let path = entry?.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|err| TestCaseError::fail(err.to_string()))?
                .to_path_buf();
            if fs::symlink_metadata(&path)?.is_dir() {
                pending.push(path);
                entries.insert(relative, None);
            } else {
                entries.insert(relative, Some(fs::read(&path)?));
            }
        }
    }
    Ok(entries)
}

fn files(snapshot: &Snapshot) -> BTreeMap<&PathBuf, &Vec<u8>> {
    snapshot
        .iter()
        .filter_map(|(path, bytes)| bytes.as_ref().map(|bytes| (path, bytes)))
        .collect()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn path_key(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        text.to_ascii_lowercase()
    } else {
        text
    }
}

fn json(output: &Output) -> Result<Value, TestCaseError> {
    serde_json::from_slice(&output.stdout).map_err(|err| TestCaseError::fail(err.to_string()))
}

#[derive(Default)]
pub(crate) struct Observed {
    pub(crate) reviewable_install_failed: Vec<String>,
    pub(crate) installs: usize,
}

fn check_install(
    world: &World,
    package: char,
    count: usize,
    observed: &mut Observed,
) -> Result<Option<(String, BTreeSet<PathBuf>, BTreeSet<PathBuf>)>, TestCaseError> {
    let before = snapshot(&world.root)?;
    let state_before = (world.state.exists(), snapshot(&world.state)?);
    let dry = world.dry_run(package)?;
    prop_assert_eq!(
        &snapshot(&world.root)?,
        &before,
        "dry run of {} changed the world",
        package
    );
    prop_assert_eq!(
        &(world.state.exists(), snapshot(&world.state)?),
        &state_before,
        "dry run of {} changed manager state",
        package
    );
    let preview = if dry.status.success() {
        Some(json(&dry)?)
    } else {
        None
    };
    let reviewable = preview
        .as_ref()
        .is_some_and(|preview| preview["status"] == "reviewable");
    let installed = world.install(package)?;
    let after = snapshot(&world.root)?;
    if !installed.status.success() {
        prop_assert_eq!(
            &after,
            &before,
            "failed install of {} left changes",
            package
        );
        if reviewable {
            observed
                .reviewable_install_failed
                .push(format!("package {package}: {}", stderr(&installed)));
        }
        return Ok(None);
    }
    observed.installs += 1;
    prop_assert!(
        reviewable,
        "install of {} succeeded after a blocked dry run",
        package
    );
    let preview = preview.expect("reviewable implies a parsed preview");
    let operations = preview["operations"]
        .as_array()
        .ok_or_else(|| TestCaseError::fail("preview has no operations"))?;
    let (before_files, after_files) = (files(&before), files(&after));
    let destinations: BTreeSet<PathBuf> = operations
        .iter()
        .filter_map(|operation| operation["destination"].as_str())
        .map(|destination| Path::new("game").join(destination))
        .collect();
    let destination_keys: BTreeSet<String> =
        destinations.iter().map(|path| path_key(path)).collect();
    for (path, bytes) in &before_files {
        if destination_keys.contains(&path_key(path)) {
            continue;
        }
        prop_assert_eq!(
            after_files.get(path),
            Some(bytes),
            "install of {} changed {:?}",
            package,
            path
        );
    }
    let added: BTreeSet<PathBuf> = after_files
        .keys()
        .filter(|path| !before_files.contains_key(*path))
        .map(|path| (*path).clone())
        .collect();
    for path in &destinations {
        prop_assert!(
            after_files.contains_key(path)
                || after_files
                    .keys()
                    .any(|actual| path_key(actual) == path_key(path)),
            "install of {} did not write the previewed {:?}",
            package,
            path
        );
    }
    for path in after.keys().filter(|path| !before.contains_key(*path)) {
        prop_assert!(
            path.starts_with("game"),
            "install of {} wrote outside the game: {:?}",
            package,
            path
        );
    }
    for operation in operations {
        let destination = operation["destination"].as_str().unwrap_or_default();
        let index = (0..count)
            .find(|&index| operation["source"] == source(package, index))
            .ok_or_else(|| TestCaseError::fail("preview names an unknown source"))?;
        prop_assert_eq!(
            fs::read(world.game.join(destination)).ok(),
            Some(payload(package, index)),
            "install of {} does not match the preview at {}",
            package,
            destination
        );
    }
    let id = json(&installed)?["id"]
        .as_str()
        .ok_or_else(|| TestCaseError::fail("install reported no package id"))?
        .to_owned();
    Ok(Some((id, added, destinations)))
}

fn check_uninstall(
    world: &World,
    id: &str,
    owned: &BTreeSet<PathBuf>,
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
    let expected: BTreeMap<_, _> = files(&before)
        .into_iter()
        .filter(|(path, _)| !owned.contains(&path_key(path)))
        .collect();
    prop_assert_eq!(
        files(&after),
        expected,
        "uninstall of {} removed the wrong files",
        id
    );
    Ok(())
}

pub(crate) fn ownership_case(case: &Case, observed: &mut Observed) -> Result<(), TestCaseError> {
    let world = World::new(case)?;
    let start = snapshot(&world.root)?;
    let Some((first_id, _first_files, mut first_owned)) =
        check_install(&world, 'a', case.first.len(), observed)?
    else {
        return Ok(());
    };
    let second = if case.second.is_empty() {
        None
    } else {
        check_install(&world, 'b', case.second.len(), observed)?
    };
    if let Some((_, _, second_destinations)) = &second {
        let second_keys: BTreeSet<String> =
            second_destinations.iter().map(|path| path_key(path)).collect();
        first_owned.retain(|path| !second_keys.contains(&path_key(path)));
    }
    check_uninstall(&world, &first_id, &first_owned)?;
    if let Some((second_id, second_files, second_destinations)) = second {
        let _ = second_files;
        check_uninstall(&world, &second_id, &second_destinations)?;
    }
    prop_assert_eq!(
        snapshot(&world.root)?,
        start,
        "uninstalling every package did not restore the game"
    );
    Ok(())
}
