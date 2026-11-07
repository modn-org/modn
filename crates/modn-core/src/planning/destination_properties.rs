use super::insert_destination;
use crate::archive::validated_name;
use crate::transactions::directory_candidate_key;
use proptest::prelude::*;
use proptest::test_runner::{Config, RngSeed, TestCaseError, TestError, TestRunner};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use tempfile::tempdir;

mod single_component;

const FIXED_SEED: u64 = 0x6d6f_646e;

fn config(default_cases: u32) -> Config {
    let mut config = Config::default();
    if std::env::var_os("PROPTEST_CASES").is_none() {
        config.cases = default_cases;
    }
    if std::env::var_os("PROPTEST_RNG_SEED").is_none() {
        config.rng_seed = RngSeed::Fixed(FIXED_SEED);
    }
    config.failure_persistence = None;
    config
}

fn run<S: Strategy>(
    name: &str,
    default_cases: u32,
    strategy: S,
    test: impl Fn(S::Value) -> Result<(), TestCaseError>,
) where
    S::Value: std::fmt::Debug,
{
    let mut runner = TestRunner::new(config(default_cases));
    match runner.run(&strategy, test) {
        Ok(()) => {}
        Err(TestError::Fail(reason, value)) => {
            panic!("{name} failed: {reason}\nminimal counterexample: {value:?}")
        }
        Err(TestError::Abort(reason)) => panic!("{name} aborted: {reason}"),
    }
}

fn accepted(destination: &str) -> bool {
    destination.len() <= 1024
        && validated_name(destination).is_ok_and(|normalized| normalized == destination)
}

const HOSTILE: &[&str] = &[
    "..",
    ".",
    "",
    "CON",
    "con.archive",
    "Aux",
    "NUL.txt",
    "COM1",
    "com9.reds",
    "LPT1",
    "COM0",
    "COM\u{b9}",
    "lpt\u{b2}.reds",
    "Com\u{b3}.archive",
    "x.",
    "x ",
    " x",
    "x..",
    "...",
    "a:b",
    "a::$DATA",
    "C:",
    "a*b",
    "a?b",
    "a<b",
    "a\"b",
    "a|b",
    "a\\b",
    "~1",
    "PROGRA~1",
    "\0",
    "\t",
    "a\u{7f}",
    "\u{202e}x",
];
const NAMES: &[&str] = &[
    "archive",
    "pc",
    "mod",
    "r6",
    "scripts",
    "bin",
    "x64",
    "a.archive",
    "x.reds",
    "mod\u{3a3}",
    "mod\u{3c3}",
    "mod\u{3c2}",
    "\u{3a3}\u{391}\u{3a3}",
    "stra\u{df}e",
    "STRASSE",
    "\u{130}",
    "\u{131}",
    "\u{212a}",
    "k",
    "\u{b5}",
    "\u{3bc}",
    "\u{17f}",
];

fn component() -> impl Strategy<Value = String> {
    prop_oneof![
        12 => prop::sample::select(NAMES).prop_map(String::from),
        1 => prop::sample::select(HOSTILE).prop_map(String::from),
        2 => prop::collection::vec(
            prop::sample::select(&[
                'a', 'B', 'z', '0', '.', ' ', '-', '_', '~', ':', '\\', '\u{3a3}', '\u{3c3}',
                '\u{3c2}', '\u{df}', '\u{130}', '\u{131}', '\u{212a}', '\u{1c5}', '\u{e9}',
            ][..]),
            1..7,
        )
        .prop_map(|chars| chars.into_iter().collect()),
    ]
}

fn destination() -> impl Strategy<Value = String> {
    (
        prop::bool::weighted(0.05),
        prop::collection::vec(component(), 1..5),
        prop::bool::weighted(0.05),
    )
        .prop_map(|(leading, parts, trailing)| {
            let mut path = parts.join("/");
            if leading {
                path.insert(0, '/');
            }
            if trailing {
                path.push('/');
            }
            path
        })
}

fn native(destination: &str) -> PathBuf {
    destination.split('/').collect()
}

fn files_under(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            } else {
                let relative = entry.path().strip_prefix(root).map(Path::to_path_buf);
                found.push(relative.unwrap_or_else(|_| entry.path()));
            }
        }
    }
    Ok(found)
}

#[test]
fn accepted_destinations_stay_inside_the_game() {
    let accepted_count = std::cell::Cell::new(0usize);
    run("containment", 512, destination(), |destination| {
        if !accepted(&destination) {
            return Ok(());
        }
        accepted_count.set(accepted_count.get() + 1);
        prop_assert!(Path::new(&destination)
            .components()
            .all(|part| matches!(part, Component::Normal(_))));
        let scratch = tempdir()?;
        let game = scratch.path().join("game");
        fs::create_dir(&game)?;
        let target = game.join(&destination);
        #[cfg(windows)]
        prop_assert_eq!(
            std::path::absolute(&target)?,
            game.join(native(&destination))
        );
        fs::create_dir_all(
            target
                .parent()
                .expect("an accepted destination has a parent"),
        )?;
        fs::write(&target, b"owned")?;
        prop_assert_eq!(
            files_under(scratch.path())?,
            vec![Path::new("game").join(native(&destination))]
        );
        Ok(())
    });
    let accepted_count = accepted_count.get();
    println!("containment: {accepted_count} accepted destinations checked");
    assert!(
        accepted_count >= 50,
        "too few accepted destinations: {accepted_count}"
    );
}

fn documented_device_name(component: &str, digits: &[char]) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|rest| {
                let mut chars = rest.chars();
                chars.next().is_some_and(|digit| digits.contains(&digit)) && chars.next().is_none()
            })
        })
}

fn documented_bad_name(component: &str) -> bool {
    component.ends_with([' ', '.'])
        || component
            .chars()
            .any(|ch| "<>:\"/\\|?*".contains(ch) || u32::from(ch) < 32)
}

fn check_documented_names(name: &str, digits: &'static [char]) {
    let checked = std::cell::Cell::new(0usize);
    run(name, 512, destination(), |destination| {
        if !accepted(&destination) {
            return Ok(());
        }
        checked.set(checked.get() + 1);
        for component in destination.split('/') {
            prop_assert!(
                !documented_bad_name(component) && !documented_device_name(component, digits),
                "{:?} is accepted, but Microsoft's naming rules forbid {:?}",
                destination,
                component
            );
        }
        Ok(())
    });
    let checked = checked.get();
    println!("{name}: {checked} accepted destinations checked");
    assert!(checked >= 50, "too few accepted destinations: {checked}");
}

#[test]
fn accepted_destinations_follow_documented_windows_names() {
    check_documented_names(
        "documented names",
        &['1', '2', '3', '4', '5', '6', '7', '8', '9'],
    );
}

#[test]
fn accepted_destinations_avoid_superscript_device_names() {
    check_documented_names("superscript device names", &['\u{b9}', '\u{b2}', '\u{b3}']);
}

fn spelling_of(destination: &str, choices: &[u8]) -> String {
    let partners = |ch: char| -> Vec<String> {
        let mut options = vec![
            ch.to_string(),
            ch.to_uppercase().collect(),
            ch.to_lowercase().collect(),
        ];
        for group in [
            &["\u{3a3}", "\u{3c3}", "\u{3c2}"][..],
            &["k", "K", "\u{212a}"],
            &["i", "I", "\u{130}", "\u{131}"],
            &["\u{b5}", "\u{3bc}", "\u{39c}"],
            &["s", "S", "\u{17f}"],
            &["\u{df}", "ss", "SS", "\u{1e9e}"],
        ] {
            if group.contains(&ch.to_string().as_str()) {
                options.extend(group.iter().map(|item| (*item).to_owned()));
            }
        }
        options
    };
    destination
        .chars()
        .zip(choices.iter().cycle())
        .map(|(ch, &choice)| {
            let options = partners(ch);
            options[usize::from(choice) % options.len()].clone()
        })
        .collect()
}

#[test]
fn same_file_spellings_share_one_ownership_key() {
    let same_file_pairs = std::cell::Cell::new(0usize);
    let strategy = (destination(), prop::collection::vec(any::<u8>(), 1..16));
    run("ownership key", 1024, strategy, |(first, choices)| {
        let second = spelling_of(&first, &choices);
        if first == second || !accepted(&first) || !accepted(&second) {
            return Ok(());
        }
        let scratch = tempdir()?;
        let target = scratch.path().join(&first);
        fs::create_dir_all(
            target
                .parent()
                .expect("an accepted destination has a parent"),
        )?;
        fs::write(&target, b"owned")?;
        let same_file =
            fs::symlink_metadata(scratch.path().join(&second)).is_ok_and(|meta| meta.is_file());
        if !same_file {
            return Ok(());
        }
        same_file_pairs.set(same_file_pairs.get() + 1);
        let mut reserved = BTreeSet::new();
        prop_assert_eq!(insert_destination(&mut reserved, "game", &first), None);
        prop_assert!(
            insert_destination(&mut reserved, "game", &second).is_some(),
            "{:?} and {:?} are one file, but the ownership keys are {:?} and {:?}",
            first,
            second,
            first.to_lowercase(),
            second.to_lowercase()
        );
        prop_assert_eq!(
            directory_candidate_key(&first),
            directory_candidate_key(&second)
        );
        Ok(())
    });
    let same_file_pairs = same_file_pairs.get();
    println!("ownership key: {same_file_pairs} same-file spellings checked");
    if cfg!(windows) {
        assert!(
            same_file_pairs >= 50,
            "too few same-file pairs: {same_file_pairs}"
        );
    }
}

#[test]
fn destinations_are_unique_per_root_and_fail_closed_across_roots() {
    let mut reserved = BTreeSet::new();
    assert_eq!(
        insert_destination(&mut reserved, "game", "Mods/core.lua"),
        None
    );
    assert_eq!(
        insert_destination(&mut reserved, "content", "Mods/core.lua"),
        None,
        "one relative path may exist below two roots"
    );
    assert_eq!(
        insert_destination(&mut reserved, "content", "mods/CORE.lua"),
        if cfg!(windows) {
            Some("DUPLICATE_DESTINATION")
        } else {
            None
        },
        "content-root keys use the same platform case folding"
    );
    assert_eq!(
        insert_destination(&mut reserved, "content", "Mods"),
        Some("FILE_DIRECTORY_COLLISION")
    );
}
