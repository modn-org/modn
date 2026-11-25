#![allow(
    dead_code,
    reason = "shared fixture exposes helpers for different test crates"
)]

use std::fs;
use std::path::{Path, PathBuf};

#[path = "git_command.rs"]
mod git_command;

pub struct Repository {
    pub base: PathBuf,
    pub commit: String,
    pub later: String,
}

pub fn git(directory: &Path, arguments: &[&str]) -> String {
    let output = git_command::git_command()
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git output")
        .trim()
        .to_owned()
}

pub fn commit_all(directory: &Path, message: &str) {
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
}

pub fn repository(root: &Path) -> Repository {
    let work = root.join("git-source-work");
    let plugin = work.join("Demo/package/plugins/git-source.plugin");
    fs::create_dir_all(plugin.parent().expect("plugin parent")).expect("work tree");
    fs::write(&plugin, b"pinned-release").expect("pinned payload");
    git(&work, &["init", "--initial-branch=main"]);
    commit_all(&work, "pinned");
    let commit = git(&work, &["rev-parse", "HEAD"]);
    fs::write(&plugin, b"later-release").expect("later payload");
    commit_all(&work, "later");
    let later = git(&work, &["rev-parse", "HEAD"]);
    let base = root.join("repos");
    fs::create_dir_all(base.join("fixture")).expect("remote base");
    git(
        root,
        &[
            "clone",
            "--bare",
            "--no-local",
            "--",
            work.to_str().expect("work path"),
            base.join("fixture/project.git")
                .to_str()
                .expect("bare path"),
        ],
    );
    Repository {
        base,
        commit,
        later,
    }
}

pub fn file_url(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    }
}
