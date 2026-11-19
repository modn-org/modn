//! Resolution rules of the `modn-theme` protocol, tested without Tauri.
//!
//! The module under test has no Tauri dependency, so these tests run on every
//! platform. The fixture creates every file under the test target directory.

#[expect(
    dead_code,
    reason = "ASSET_BASE belongs to the command layer and these tests never read it"
)]
#[path = "../src/app/theme_assets.rs"]
mod theme_assets;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use theme_assets::{ASSET_LIMIT, load_asset, resolve_asset};

/// A disposable theme directory with an `assets` directory.
struct Fixture {
    base: PathBuf,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(1);
        let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "modn-desktop-theme-assets-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let root = base.join("theme");
        fs::create_dir_all(root.join("assets/icons")).expect("create the theme fixture");
        Self { base, root }
    }

    fn write(&self, relative: &str, bytes: &[u8]) {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create the asset folder");
        }
        fs::write(path, bytes).expect("write the asset");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn unresolved(fixture: &Fixture, request: &str) -> bool {
    resolve_asset(&fixture.root, request).is_err()
}

#[test]
fn a_nested_asset_resolves_with_its_content_type() {
    let fixture = Fixture::new();
    fixture.write("assets/icons/night.png", b"\x89PNG fixture");
    let asset = resolve_asset(&fixture.root, "/assets/icons/night.png").expect("nested asset");
    assert_eq!(asset.content_type, "image/png");
    assert_eq!(
        asset.path,
        fs::canonicalize(fixture.root.join("assets/icons/night.png")).expect("canonical path")
    );
}

#[test]
fn the_three_documented_extensions_map_to_their_types() {
    let fixture = Fixture::new();
    for (name, content_type) in [
        ("a.png", "image/png"),
        ("b.webp", "image/webp"),
        ("c.woff2", "font/woff2"),
    ] {
        fixture.write(&format!("assets/{name}"), b"fixture");
        let asset = resolve_asset(&fixture.root, &format!("/assets/{name}")).expect(name);
        assert_eq!(asset.content_type, content_type);
    }
}

#[test]
fn a_parent_segment_is_refused() {
    let fixture = Fixture::new();
    fixture.write("assets/icon.png", b"fixture");
    assert!(unresolved(&fixture, "/assets/../theme.css"));
    assert!(unresolved(&fixture, "/assets/icons/../../theme.css"));
    assert!(unresolved(&fixture, "/assets/../assets/icon.png"));
}

#[test]
fn an_encoded_parent_segment_is_refused() {
    let fixture = Fixture::new();
    fixture.write("assets/icon.png", b"fixture");
    assert!(unresolved(&fixture, "/assets/%2e%2e/theme.css"));
    assert!(unresolved(&fixture, "/assets/%2E%2E/theme.css"));
    assert!(unresolved(&fixture, "/assets/%2e%2e/assets/icon.png"));
}

#[test]
fn an_encoded_separator_is_refused() {
    let fixture = Fixture::new();
    assert!(unresolved(&fixture, "/assets/icons%2f..%2f..%2ftheme.css"));
    assert!(unresolved(&fixture, "/assets/icons%5c..%5c..%5ctheme.css"));
}

#[test]
fn a_raw_backslash_is_refused() {
    let fixture = Fixture::new();
    assert!(unresolved(&fixture, "/assets\\..\\theme.css"));
    assert!(unresolved(&fixture, "/assets/icons\\night.png"));
}

#[test]
fn an_uppercase_name_is_refused() {
    let fixture = Fixture::new();
    fixture.write("assets/Night.png", b"fixture");
    assert!(unresolved(&fixture, "/assets/Night.png"));
}

#[test]
fn an_unknown_extension_is_refused() {
    let fixture = Fixture::new();
    fixture.write("assets/theme.css", b"body{}");
    fixture.write("assets/icon.svg", b"<svg/>");
    fixture.write("assets/icon.png.exe", b"fixture");
    fixture.write("assets/icon", b"fixture");
    for request in [
        "/assets/theme.css",
        "/assets/icon.svg",
        "/assets/icon.png.exe",
        "/assets/icon",
    ] {
        assert!(unresolved(&fixture, request), "{request} must be refused");
    }
}

#[test]
fn a_path_outside_assets_is_refused() {
    let fixture = Fixture::new();
    fixture.write("theme.css", b"body{}");
    fixture.write("icon.png", b"fixture");
    for request in ["/theme.css", "/icon.png", "/assets", "/assetsx/icon.png"] {
        assert!(unresolved(&fixture, request), "{request} must be refused");
    }
}

#[test]
fn the_root_and_empty_paths_are_refused() {
    let fixture = Fixture::new();
    for request in ["/", "", "/assets/", "/assets"] {
        assert!(unresolved(&fixture, request), "{request} must be refused");
    }
}

#[test]
fn absolute_windows_paths_are_refused() {
    let fixture = Fixture::new();
    for request in [
        r"C:\Windows\win.png",
        "C:/Windows/win.png",
        r"\\server\share\a.png",
        "/assets/C:/Windows/a.png",
    ] {
        assert!(unresolved(&fixture, request), "{request} must be refused");
    }
}

#[test]
fn a_directory_is_refused() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.root.join("assets/folder.png")).expect("create the directory");
    assert!(unresolved(&fixture, "/assets/folder.png"));
}

#[test]
fn an_oversize_asset_is_refused() {
    let fixture = Fixture::new();
    fixture.write("assets/large.png", &vec![0; ASSET_LIMIT as usize + 1]);
    fixture.write("assets/exact.png", &vec![0; ASSET_LIMIT as usize]);
    assert!(unresolved(&fixture, "/assets/large.png"));
    assert!(resolve_asset(&fixture.root, "/assets/exact.png").is_ok());
}

#[test]
fn a_dot_segment_and_a_double_separator_are_refused() {
    let fixture = Fixture::new();
    fixture.write("assets/icon.png", b"fixture");
    assert!(unresolved(&fixture, "/assets/./icon.png"));
    assert!(unresolved(&fixture, "/assets//icon.png"));
}

#[test]
fn a_malformed_escape_is_refused() {
    let fixture = Fixture::new();
    fixture.write("assets/icon.png", b"fixture");
    for request in ["/assets/%zz.png", "/assets/%2.png", "/assets/icon%2.png"] {
        assert!(unresolved(&fixture, request), "{request} must be refused");
    }
}

#[test]
fn an_encoded_nul_is_refused() {
    let fixture = Fixture::new();
    assert!(unresolved(&fixture, "/assets/icon%00.png"));
}

#[test]
fn a_missing_asset_and_a_missing_root_are_refused() {
    let fixture = Fixture::new();
    assert!(unresolved(&fixture, "/assets/missing.png"));
    let missing = fixture.base.join("absent");
    assert!(resolve_asset(&missing, "/assets/icon.png").is_err());
}

#[cfg(unix)]
#[test]
fn a_symlinked_component_is_refused() {
    let fixture = Fixture::new();
    let outside = fixture.base.join("outside.png");
    fs::write(&outside, b"fixture").expect("write the outside file");
    std::os::unix::fs::symlink(&outside, fixture.root.join("assets/link.png"))
        .expect("create the symlink");
    std::os::unix::fs::symlink(
        &fixture.root.join("assets/icons"),
        fixture.root.join("assets/linked"),
    )
    .expect("create the directory symlink");
    assert!(unresolved(&fixture, "/assets/link.png"));
    assert!(unresolved(&fixture, "/assets/linked/night.png"));
}

#[cfg(not(unix))]
#[test]
fn a_symlinked_component_is_refused() {
    // Creating a symlink on Windows needs Developer Mode or elevation, which a
    // test must not require. The same check runs on the Unix CI leg.
    eprintln!("skipped: creating a symlink needs a privilege this test does not request");
}

#[test]
fn a_request_without_a_selected_theme_answers_404() {
    let response = load_asset(None, "GET", "/assets/icon.png");
    assert_eq!(response.status, 404);
    assert!(response.body.is_empty());
    assert!(response.content_type.is_none());
}

#[test]
fn a_get_of_a_resolved_asset_answers_200_with_its_body() {
    let fixture = Fixture::new();
    fixture.write("assets/icon.png", b"\x89PNG fixture");
    let response = load_asset(Some(&fixture.root), "GET", "/assets/icon.png");
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type, Some("image/png"));
    assert_eq!(response.body, b"\x89PNG fixture".to_vec());
}

#[test]
fn a_refused_path_answers_404_without_a_reason() {
    let fixture = Fixture::new();
    let response = load_asset(Some(&fixture.root), "GET", "/assets/../theme.css");
    assert_eq!(response.status, 404);
    assert!(response.body.is_empty());
    assert!(response.content_type.is_none());
}

#[test]
fn methods_other_than_get_answer_405() {
    let fixture = Fixture::new();
    fixture.write("assets/icon.png", b"fixture");
    for method in ["POST", "PUT", "HEAD", "OPTIONS", "DELETE"] {
        let response = load_asset(Some(&fixture.root), method, "/assets/icon.png");
        assert_eq!(response.status, 405, "{method} must not be served");
        assert!(response.body.is_empty());
    }
}
