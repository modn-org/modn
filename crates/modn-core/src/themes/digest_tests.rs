use super::theme_digest;
use crate::themes::Rule;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

fn write(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
}

fn hex_digest(dir: &Path) -> String {
    theme_digest(dir).unwrap()
}

#[cfg(unix)]
fn link_file(original: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(original, link)
}

#[cfg(windows)]
fn link_file(original: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(original, link)
}

#[test]
fn a_fixed_tree_has_a_pinned_digest() {
    let temp = tempdir().unwrap();
    let dir = temp.path().join("theme");
    write(&dir.join("a.txt"), b"hello");
    write(&dir.join("sub/b.bin"), &[0, 1, 2, 3]);
    assert_eq!(
        hex_digest(&dir),
        "31e2b176e5b1507530e17336f7eb17777eccd9d6991928ecf9639f66f8e55ba7"
    );
}

#[test]
fn one_changed_byte_changes_the_digest() {
    let temp = tempdir().unwrap();
    let first = temp.path().join("first");
    write(&first.join("theme.css"), b".modn-app { color: #fff; }");
    let before = hex_digest(&first);
    write(&first.join("theme.css"), b".modn-app { color: #000; }");
    assert_ne!(before, hex_digest(&first));
}

#[test]
fn one_changed_file_name_changes_the_digest() {
    let temp = tempdir().unwrap();
    let first = temp.path().join("first");
    write(&first.join("a.txt"), b"same");
    let before = hex_digest(&first);
    fs::rename(first.join("a.txt"), first.join("b.txt")).unwrap();
    assert_ne!(before, hex_digest(&first));
}

#[test]
fn the_digest_does_not_depend_on_creation_order() {
    let temp = tempdir().unwrap();
    let left = temp.path().join("left");
    write(&left.join("z.txt"), b"last");
    write(&left.join("a/first.txt"), b"first");
    let right = temp.path().join("right");
    write(&right.join("a/first.txt"), b"first");
    write(&right.join("z.txt"), b"last");
    assert_eq!(hex_digest(&left), hex_digest(&right));
}

#[test]
fn the_digest_is_64_lowercase_hex_characters() {
    let temp = tempdir().unwrap();
    let dir = temp.path().join("theme");
    write(&dir.join("theme.css"), b"x");
    let digest = hex_digest(&dir);
    assert_eq!(digest.len(), 64);
    assert!(
        digest
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')),
        "{digest}"
    );
}

#[test]
fn the_digest_covers_the_relative_path_not_only_the_content() {
    let temp = tempdir().unwrap();
    let first = temp.path().join("first");
    write(&first.join("a.txt"), b"same");
    let second = temp.path().join("second");
    write(&second.join("b.txt"), b"same");
    assert_ne!(hex_digest(&first), hex_digest(&second));
}

#[test]
fn links_are_refused() {
    let temp = tempdir().unwrap();
    let dir = temp.path().join("theme");
    write(&dir.join("real.txt"), b"x");
    if link_file(&dir.join("real.txt"), &dir.join("link.txt")).is_err() {
        eprintln!("skipping link test: this platform refuses to create symlinks");
        return;
    }
    let problem = theme_digest(&dir).expect_err("a link must be refused");
    assert_eq!(problem.rule, Rule::UnexpectedFile);
    assert!(problem.message.contains("link"), "{}", problem.message);
}

#[test]
fn a_missing_directory_is_reported() {
    let temp = tempdir().unwrap();
    let problem = theme_digest(&temp.path().join("missing")).expect_err("missing directory");
    assert_eq!(problem.rule, Rule::ThemeMissing);
}

#[test]
fn a_file_instead_of_a_directory_is_reported() {
    let temp = tempdir().unwrap();
    let file = temp.path().join("theme");
    fs::write(&file, b"not a directory").unwrap();
    let problem = theme_digest(&file).expect_err("a file is not a theme directory");
    assert_eq!(problem.rule, Rule::ThemeMissing);
}
