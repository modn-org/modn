#[path = "support/main_bucket.rs"]
mod main_bucket;
use proptest::prelude::*;
use proptest::test_runner::{Config, RngSeed, TestCaseError, TestError, TestRunner};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::{tempdir, TempDir};
use zip::write::SimpleFileOptions;

const FIXED_SEED: u64 = 0x6f77_6e65;

fn config(default_cases: u32) -> Config {
    let mut config = Config::default();
    if std::env::var_os("PROPTEST_CASES").is_none() {
        config.cases = default_cases;
    }
    if std::env::var_os("PROPTEST_RNG_SEED").is_none() {
        config.rng_seed = RngSeed::Fixed(FIXED_SEED);
    }
    config.failure_persistence = None;
    config.max_shrink_iters = 64;
    config
}

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
struct Case {
    user_files: Vec<String>,
    first: Vec<String>,
    second: Vec<String>,
}

fn case() -> impl Strategy<Value = Case> {
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
        fs::create_dir_all(&module)?;
        let bundled =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main/games/cyberpunk-2077");
        fs::copy(bundled.join("module.json"), module.join("module.json"))?;
        let code = fs::read_to_string(bundled.join("module.lua"))?.replace("\r\n", "\n");
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
        Ok(Command::new(env!("CARGO_BIN_EXE_modn"))
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

fn json(output: &Output) -> Result<Value, TestCaseError> {
    serde_json::from_slice(&output.stdout).map_err(|err| TestCaseError::fail(err.to_string()))
}

#[derive(Default)]
struct Observed {
    reviewable_install_failed: Vec<String>,
    installs: usize,
}

fn check_install(
    world: &World,
    package: char,
    count: usize,
    observed: &mut Observed,
) -> Result<Option<(String, BTreeSet<PathBuf>)>, TestCaseError> {
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
    for (path, bytes) in &before_files {
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
    prop_assert_eq!(
        added.len(),
        operations.len(),
        "install of {} wrote {:?}",
        package,
        added
    );
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
    Ok(Some((id, added)))
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
    let expected: BTreeMap<_, _> = files(&before)
        .into_iter()
        .filter(|(path, _)| !owned.contains(*path))
        .collect();
    prop_assert_eq!(
        files(&after),
        expected,
        "uninstall of {} removed the wrong files",
        id
    );
    Ok(())
}

fn ownership_case(case: &Case, observed: &mut Observed) -> Result<(), TestCaseError> {
    let world = World::new(case)?;
    let start = snapshot(&world.root)?;
    let Some((first_id, first_files)) = check_install(&world, 'a', case.first.len(), observed)?
    else {
        return Ok(());
    };
    let second = if case.second.is_empty() {
        None
    } else {
        check_install(&world, 'b', case.second.len(), observed)?
    };
    check_uninstall(&world, &first_id, &first_files)?;
    if let Some((second_id, second_files)) = second {
        check_uninstall(&world, &second_id, &second_files)?;
    }
    prop_assert_eq!(
        snapshot(&world.root)?,
        start,
        "uninstalling every package did not restore the game"
    );
    Ok(())
}

fn run_cases(default_cases: u32, check: impl Fn(&Observed) -> Result<(), TestCaseError>) -> usize {
    let installs = std::cell::Cell::new(0usize);
    let mut runner = TestRunner::new(config(default_cases));
    let result = runner.run(&case(), |case| {
        let mut observed = Observed::default();
        ownership_case(&case, &mut observed)?;
        installs.set(installs.get() + observed.installs);
        check(&observed)
    });
    match result {
        Ok(()) => installs.get(),
        Err(TestError::Fail(reason, case)) => {
            panic!("{reason}\nminimal counterexample: {case:#?}")
        }
        Err(TestError::Abort(reason)) => panic!("aborted: {reason}"),
    }
}

#[test]
fn dry_run_install_and_uninstall_keep_ownership_promises() {
    let installs = run_cases(64, |_| Ok(()));
    println!("ownership: {installs} approved installs checked");
    assert!(installs >= 5, "too few approved installs: {installs}");
}

#[test]
fn single_component_destination_plans_installs_and_uninstalls() {
    let case = Case {
        user_files: Vec::new(),
        first: vec!["single.lua".into(), "Mods/inner.lua".into()],
        second: Vec::new(),
    };
    let mut observed = Observed::default();
    let result = ownership_case(&case, &mut observed);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(observed.installs, 1);
}

#[test]
fn reviewable_dry_run_predicts_the_install() {
    let installs = run_cases(256, |observed| {
        prop_assert!(
            observed.reviewable_install_failed.is_empty(),
            "reviewable dry run, failed install: {:?}",
            observed.reviewable_install_failed
        );
        Ok(())
    });
    println!("preview agreement: {installs} approved installs checked");
    assert!(installs >= 5, "too few approved installs: {installs}");
}
