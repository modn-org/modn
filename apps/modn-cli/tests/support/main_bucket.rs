#![allow(
    dead_code,
    reason = "shared fixture exposes helpers for different test crates"
)]
use std::ffi::OsStr;
use std::fs;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use tempfile::TempDir;

/// The `modn` binary that tests run.
///
/// Windows locks a running executable, so a concurrent `cargo build` cannot
/// replace `target/debug/modn.exe` while tests run it. On Windows, each test
/// crate runs its own copy instead.
pub fn modn_binary() -> PathBuf {
    let source = Path::new(env!("CARGO_BIN_EXE_modn"));
    if !cfg!(windows) {
        return source.to_owned();
    }
    static COPY: OnceLock<PathBuf> = OnceLock::new();
    COPY.get_or_init(|| crate_copy(source)).clone()
}

fn crate_copy(source: &Path) -> PathBuf {
    let test_crate = std::env::current_exe().expect("test executable");
    let folder = source
        .parent()
        .expect("binary folder")
        .join("test-binaries")
        .join(test_crate.file_stem().expect("test executable name"));
    let copy = folder.join(source.file_name().expect("binary name"));
    if !same_build(source, &copy) {
        fs::create_dir_all(&folder).expect("binary copy folder");
        fs::copy(source, &copy).expect("copy of the modn binary");
    }
    copy
}

fn same_build(source: &Path, copy: &Path) -> bool {
    let (Ok(source), Ok(copy)) = (fs::metadata(source), fs::metadata(copy)) else {
        return false;
    };
    source.len() == copy.len() && source.modified().ok() == copy.modified().ok()
}

fn program(binary: &OsStr) -> PathBuf {
    if Path::new(binary) == Path::new(env!("CARGO_BIN_EXE_modn")) {
        modn_binary()
    } else {
        PathBuf::from(binary)
    }
}

pub fn seed(data: &Path) {
    fn copy(source: &Path, target: &Path) {
        fs::create_dir_all(target).expect("bucket directory");
        for entry in fs::read_dir(source).expect("bucket entries") {
            let entry = entry.expect("bucket entry");
            let target = target.join(entry.file_name());
            if entry.file_type().expect("entry type").is_dir() {
                copy(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).expect("bucket file");
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
    let mut command = Command::new(program(binary.as_ref()));
    command.env("LOCALAPPDATA", data.path());
    SeededCommand {
        command,
        _data: data,
    }
}
