use super::{
    accepted, component, documented_bad_name, documented_device_name, files_under, run, spelling_of,
};
use crate::planning::insert_destination;
use crate::transactions::directory_candidate_key;
use proptest::prelude::*;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path};
use tempfile::tempdir;

const SINGLE_PREFIXES: &[&str] = &[
    "",
    "",
    "",
    "",
    "",
    "",
    "/",
    "./",
    "../",
    "C:",
    "c:/",
    "//server/share/",
    "\\\\server\\share\\",
    "\\\\?\\C:\\",
    "\\\\.\\",
];
const SINGLE_SUFFIXES: &[&str] = &[
    "", "", "", "", ".lua", ".lua", ".", " ", ". ", "/", "::$DATA", ":x", "\\", "/..",
];
const SINGLE_DEVICES: &[&str] = &[
    "CON",
    "prn",
    "Aux",
    "nul",
    "COM1",
    "com9",
    "LPT1",
    "lpt9",
    "COM\u{b9}",
    "LPT\u{b3}",
];

fn single_component() -> impl Strategy<Value = String> {
    let name = prop_oneof![
        8 => component(),
        2 => prop::sample::select(SINGLE_DEVICES).prop_map(String::from),
    ];
    (
        prop::sample::select(SINGLE_PREFIXES),
        name,
        prop::sample::select(SINGLE_SUFFIXES),
    )
        .prop_map(|(prefix, name, suffix)| format!("{prefix}{name}{suffix}"))
}

#[test]
fn single_component_destinations_keep_every_rule() {
    let accepted_count = std::cell::Cell::new(0usize);
    let rejected_count = std::cell::Cell::new(0usize);
    let same_file_pairs = std::cell::Cell::new(0usize);
    let strategy = (
        single_component(),
        prop::collection::vec(any::<u8>(), 1..16),
    );
    run(
        "single component",
        1024,
        strategy,
        |(destination, choices)| {
            if !accepted(&destination) {
                rejected_count.set(rejected_count.get() + 1);
                return Ok(());
            }
            accepted_count.set(accepted_count.get() + 1);
            prop_assert_eq!(
                Path::new(&destination).components().collect::<Vec<_>>(),
                vec![Component::Normal(destination.as_ref())]
            );
            for digits in [
                &['1', '2', '3', '4', '5', '6', '7', '8', '9'][..],
                &['\u{b9}', '\u{b2}', '\u{b3}'],
            ] {
                prop_assert!(
                    !documented_bad_name(&destination)
                        && !documented_device_name(&destination, digits),
                    "{:?} is accepted, but Microsoft's naming rules forbid it",
                    destination
                );
            }
            let scratch = tempdir()?;
            let game = scratch.path().join("game");
            fs::create_dir(&game)?;
            fs::write(game.join(&destination), b"owned")?;
            prop_assert_eq!(
                files_under(scratch.path())?,
                vec![Path::new("game").join(&destination)]
            );
            let second = spelling_of(&destination, &choices);
            if second == destination
                || !accepted(&second)
                || !fs::symlink_metadata(game.join(&second)).is_ok_and(|meta| meta.is_file())
            {
                return Ok(());
            }
            same_file_pairs.set(same_file_pairs.get() + 1);
            let mut reserved = BTreeSet::new();
            prop_assert_eq!(insert_destination(&mut reserved, &destination), None);
            prop_assert!(
                insert_destination(&mut reserved, &second).is_some(),
                "{:?} and {:?} are one file, but their ownership keys differ",
                destination,
                second
            );
            prop_assert_eq!(
                directory_candidate_key(&destination),
                directory_candidate_key(&second)
            );
            Ok(())
        },
    );
    let (accepted, rejected, pairs) = (
        accepted_count.get(),
        rejected_count.get(),
        same_file_pairs.get(),
    );
    println!(
        "single component: {accepted} accepted, {rejected} rejected, {pairs} same-file spellings checked"
    );
    assert!(accepted >= 100, "too few accepted destinations: {accepted}");
    assert!(rejected >= 100, "too few rejected destinations: {rejected}");
    if cfg!(windows) {
        assert!(pairs >= 20, "too few same-file pairs: {pairs}");
    }
}

#[test]
fn single_component_unsafe_destinations_stay_rejected() {
    for destination in [
        "",
        ".",
        "..",
        "...",
        "/",
        "/x.lua",
        "x.lua/",
        "./x.lua",
        "../x.lua",
        "C:",
        "C:x.lua",
        "C:/x.lua",
        "//server/share/x.lua",
        "\\\\server\\share\\x.lua",
        "\\\\?\\C:\\x.lua",
        "x\\y.lua",
        "x.lua:stream",
        "x.lua::$DATA",
        "x.lua.",
        "x.lua ",
        "x.lua. ",
        "CON",
        "con.lua",
        "PRN",
        "AUX.txt",
        "NUL",
        "nul.tar.gz",
        "COM1",
        "com9.lua",
        "LPT1",
        "lpt9.lua",
        "COM\u{b9}",
        "lpt\u{b2}.lua",
        "a*b.lua",
        "a?b.lua",
        "a<b.lua",
        "a>b.lua",
        "a|b.lua",
        "a\"b.lua",
        "a\tb.lua",
        "\0",
        "a\u{7f}.lua",
    ] {
        assert!(!accepted(destination), "{destination:?} is accepted");
    }
    for destination in [
        "x.lua",
        "Mods.lua",
        "COM0.lua",
        "CONFIG.lua",
        "a.b.lua",
        "~1",
    ] {
        assert!(accepted(destination), "{destination:?} is rejected");
    }
}
