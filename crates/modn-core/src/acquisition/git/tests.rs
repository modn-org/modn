use super::{ArchiveCache, CacheUse, GitCacheKey, GitSource, Origin};
use crate::git::{self, GitEnvironment};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn git(directory: &Path, arguments: &[&str]) -> String {
    let hooks = git::empty_hooks().unwrap();
    git::run(
        &GitEnvironment {
            directory,
            hooks: hooks.path(),
            allow_file: true,
        },
        arguments,
    )
    .unwrap()
}

fn commit(directory: &Path, message: &str) -> String {
    git(directory, &["add", "."]);
    git(
        directory,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            message,
        ],
    );
    git(directory, &["rev-parse", "HEAD"])
}

fn bare_repository(root: &Path) -> (PathBuf, String) {
    let work = root.join("work");
    fs::create_dir_all(work.join("mod")).unwrap();
    fs::create_dir_all(work.join("plugins")).unwrap();
    fs::write(work.join("mod/info.json"), br#"{"name":"fixture"}"#).unwrap();
    fs::write(work.join("plugins/one.lua"), b"return 'one'").unwrap();
    git(&work, &["init", "--initial-branch=main"]);
    let pinned = commit(&work, "first");
    fs::write(work.join("plugins/one.lua"), b"return 'two'").unwrap();
    commit(&work, "second");
    let bare = root.join("project.git");
    git(
        root,
        &[
            "clone",
            "--bare",
            "--no-local",
            "--",
            work.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    (bare, pinned)
}

fn file_url(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    }
}

fn zip_entries(path: &Path) -> Vec<(String, Vec<u8>)> {
    let mut archive = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    (0..archive.len())
        .map(|index| {
            let mut entry = archive.by_index(index).unwrap();
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut bytes).unwrap();
            (entry.name().to_owned(), bytes)
        })
        .collect()
}

#[test]
fn pinned_commit_is_fetched_packed_and_cached() {
    let root = tempfile::tempdir().unwrap();
    let (bare, pinned) = bare_repository(root.path());
    let source = GitSource::new("fixture/project".into(), pinned.clone());
    let cache = ArchiveCache::new(root.path().join("cache"));
    let key = GitCacheKey::new("test-game", "fixture", "1.0", &pinned);

    let first = source.acquire(&cache, &key, CacheUse::Reuse).unwrap();
    assert_eq!(first.origin, Origin::Download);
    assert_eq!(first.sha256.len(), 64);
    assert!(first.path.is_file());
    assert!(
        first
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains(&format!("#git-{pinned}-")),
        "{}",
        first.path.display()
    );
    let entries = zip_entries(&first.path);
    let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["mod/info.json", "plugins/one.lua"]);
    assert_eq!(entries[1].1, b"return 'one'", "the pinned commit, not HEAD");

    let second = source.acquire(&cache, &key, CacheUse::Reuse).unwrap();
    assert_eq!(second.origin, Origin::Cache);
    assert_eq!(second.path, first.path);
    assert_eq!(second.sha256, first.sha256);

    let refreshed = source.acquire(&cache, &key, CacheUse::Refresh).unwrap();
    assert_eq!(refreshed.origin, Origin::Download);
    assert_eq!(refreshed.sha256, first.sha256);

    let staging = root.path().join("staging-one");
    fs::create_dir_all(&staging).unwrap();
    let one = source.pack_url(&file_url(&bare), &staging).unwrap();
    let staging = root.path().join("staging-two");
    fs::create_dir_all(&staging).unwrap();
    let two = source.pack_url(&file_url(&bare), &staging).unwrap();
    assert_eq!(fs::read(&one.path).unwrap(), fs::read(&two.path).unwrap());
    assert_eq!(one.sha256, two.sha256);
}

#[test]
fn an_unknown_commit_fails_before_a_cache_entry_exists() {
    let root = tempfile::tempdir().unwrap();
    let (_bare, _) = bare_repository(root.path());
    let missing = "0".repeat(40);
    let source = GitSource::new("fixture/project".into(), missing.clone());
    let cache = ArchiveCache::new(root.path().join("cache"));
    let key = GitCacheKey::new("test-game", "fixture", "1.0", &missing);
    let error = source.acquire(&cache, &key, CacheUse::Reuse).unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("GIT_SOURCE_FETCH_FAILED"), "{message}");
    assert!(
        fs::read_dir(cache.root())
            .map(|entries| entries
                .filter_map(Result::ok)
                .all(|entry| !entry.path().extension().is_some_and(|e| e == "zip")))
            .unwrap_or(true)
    );
}

fn indexed_entry(directory: &Path, mode: &str, name: &str) -> String {
    let mut child = Command::new("git")
        .args(["hash-object", "-w", "--stdin"])
        .current_dir(directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"target")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let object = String::from_utf8(output.stdout).unwrap().trim().to_owned();
    git(
        directory,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("{mode},{object},{name}"),
        ],
    );
    object
}

#[test]
fn a_tree_with_a_symlink_or_submodule_is_rejected() {
    for mode in ["120000", "160000"] {
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("work");
        fs::create_dir_all(&work).unwrap();
        fs::write(work.join("regular.txt"), b"regular").unwrap();
        git(&work, &["init", "--initial-branch=main"]);
        git(&work, &["add", "regular.txt"]);
        indexed_entry(&work, mode, "special");
        git(
            &work,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-m",
                "hostile",
            ],
        );
        let pinned = git(&work, &["rev-parse", "HEAD"]);
        assert_eq!(pinned.len(), 40);
        let bare = root.path().join("project.git");
        git(
            root.path(),
            &[
                "clone",
                "--bare",
                "--no-local",
                "--",
                work.to_str().unwrap(),
                bare.to_str().unwrap(),
            ],
        );
        let source = GitSource::new("fixture/project".into(), pinned);
        let staging = root.path().join("staging");
        fs::create_dir_all(&staging).unwrap();
        let error = source
            .pack_url(&file_url(&bare), &staging)
            .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("GIT_SOURCE_UNSUPPORTED_ENTRY"), "{message}");
        let expected = if mode == "120000" {
            "symbolic links"
        } else {
            "submodules"
        };
        assert!(message.contains(expected), "{message}");
        assert!(!staging.join("archive.zip").exists());
    }
}
