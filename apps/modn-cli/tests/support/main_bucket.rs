#![allow(
    dead_code,
    reason = "shared fixture exposes helpers for different test crates"
)]
use std::ffi::OsStr;
use std::ops::{Deref, DerefMut};
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

pub fn seed(data: &Path) {
    fn copy(source: &Path, target: &Path) {
        std::fs::create_dir_all(target).expect("bucket directory");
        for entry in std::fs::read_dir(source).expect("bucket entries") {
            let entry = entry.expect("bucket entry");
            let target = target.join(entry.file_name());
            if entry.file_type().expect("entry type").is_dir() {
                copy(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).expect("bucket file");
            }
        }
    }
    copy(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main"),
        &data.join("modn/buckets/main"),
    );
}

pub struct SeededCommand {
    command: Command,
    _data: TempDir,
}

impl Deref for SeededCommand {
    type Target = Command;
    fn deref(&self) -> &Command {
        &self.command
    }
}

impl DerefMut for SeededCommand {
    fn deref_mut(&mut self) -> &mut Command {
        &mut self.command
    }
}

pub fn command(binary: impl AsRef<OsStr>) -> SeededCommand {
    let data = tempfile::tempdir().expect("data directory");
    seed(data.path());
    let mut command = Command::new(binary);
    command.env("LOCALAPPDATA", data.path());
    SeededCommand {
        command,
        _data: data,
    }
}
