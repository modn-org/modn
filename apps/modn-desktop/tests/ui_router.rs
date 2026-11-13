//! Static checks for the desktop router.
//!
//! The app ships plain ES modules and no bundler, so these tests hold the
//! navigation promises: every route renders into its own container, and a
//! load that settles late cannot replace the screen the reader moved to.

use std::fs;
use std::path::Path;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

fn read(relative: &str) -> String {
    let path = Path::new(ROOT).join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"))
}

/// Returns the source from `marker` through its first balanced brace block.
fn braced<'a>(source: &'a str, marker: &str) -> &'a str {
    let start = source
        .find(marker)
        .unwrap_or_else(|| panic!("missing {marker}"));
    let open = start + source[start..].find('{').expect("opening brace");
    let mut depth = 0usize;
    for (offset, character) in source[open..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[start..open + offset + 1];
                }
            }
            _ => {}
        }
    }
    panic!("unterminated {marker}");
}

#[test]
fn each_navigation_renders_into_a_fresh_container() {
    let app = read("ui/js/app.js");
    let route = braced(&app, "async function renderRoute()");
    assert!(
        route.contains("document.createElement('div')"),
        "each navigation must create its own container"
    );
    assert!(
        route.contains("module.render(container, ctx)"),
        "the view must render into the navigation container"
    );
    assert!(
        !route.contains("module.render(root, ctx)") && !route.contains("module.render(mount, ctx)"),
        "views must never render into the shared mount"
    );
    assert!(
        route.contains("replaceChildren(container)"),
        "the new container must replace the previous one"
    );
}

#[test]
fn a_stale_navigation_cannot_write_to_the_current_screen() {
    let app = read("ui/js/app.js");
    let route = braced(&app, "async function renderRoute()");
    assert!(
        app.contains("let navigation = 0"),
        "the router needs a navigation token"
    );
    assert!(
        route.contains("++navigation"),
        "each navigation must take a fresh token"
    );
    assert!(
        route.contains("current === navigation"),
        "late writes must compare the navigation token"
    );
    assert!(
        route.contains("container.isConnected"),
        "late writes must check the container is still mounted"
    );
}
