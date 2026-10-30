//! Filesystem confinement and durable publication for the separate module store.
use super::loader;
use super::package::{digest, ModulePackage as Package};
use anyhow::{bail, Context, Result};
use std::fs::{self, File};
use std::io::{ErrorKind, Write};
use std::path::{Component, Path};

pub(super) fn check_path(path: &Path) -> Result<()> {
    if path.components().any(|c| c == Component::ParentDir) {
        bail!("module store path must not contain parent-directory components");
    }
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if meta.file_attributes() & 0x400 != 0 {
                        bail!("module store must not contain reparse points");
                    }
                }
                if meta.file_type().is_symlink() || !(meta.is_dir() || meta.is_file()) {
                    bail!("module store must not contain links or special files");
                }
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

pub(super) fn read_package(root: &Path, expected: &super::package::PackageInfo) -> Result<Package> {
    if expected.sha256.len() != 64
        || !expected.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        || !loader::valid_entrypoint(&expected.entrypoint)
    {
        bail!("invalid stored module record");
    }
    let directory = root.join("versions").join(&expected.sha256);
    check_path(&directory)?;
    let canonical = directory
        .canonicalize()
        .context("stored module version missing; reinstall the original ZIP or disable it")?;
    let manifest = loader::bounded_file(
        &canonical.join("module.json"),
        &canonical,
        loader::MAX_MANIFEST_BYTES,
    )?;
    let script = loader::bounded_file(
        &canonical.join(&expected.entrypoint),
        &canonical,
        loader::MAX_SCRIPT_BYTES,
    )?;
    check_path(&canonical.join("module.json"))?;
    check_path(&canonical.join(&expected.entrypoint))?;
    let manifest_digest = digest(&manifest);
    let script_digest = digest(&script);
    // Never execute even validation code until the selected bytes match their record.
    if manifest_digest != expected.manifest_sha256 || script_digest != expected.script_sha256 {
        bail!("stored module integrity mismatch: manifest expected {}, observed {}; script expected {}, observed {}; restore original bytes or explicitly disable the module",
            expected.manifest_sha256, manifest_digest, expected.script_sha256, script_digest);
    }
    let observed = Package::from_bytes(manifest, script)?;
    if &observed.info != expected {
        bail!("stored module integrity mismatch: expected content {}, manifest {}, script {}; observed content {}, manifest {}, script {}; restore original bytes or explicitly disable the module",
            expected.sha256, expected.manifest_sha256, expected.script_sha256,
            observed.info.sha256, observed.info.manifest_sha256, observed.info.script_sha256);
    }
    if fs::read_dir(&canonical)?.count() != 2 {
        bail!("stored module contains unexpected files; restore it or disable it");
    }
    Ok(observed)
}

pub(super) fn publish(root: &Path, package: &Package) -> Result<()> {
    let versions = root.join("versions");
    check_path(&versions)?;
    fs::create_dir_all(&versions)?;
    let target = versions.join(&package.info.sha256);
    if target.try_exists()? {
        read_package(root, &package.info)?;
        return Ok(());
    }
    let stage = tempfile::Builder::new()
        .prefix(".staging-")
        .tempdir_in(root)?;
    for (name, bytes) in [
        ("module.json", &package.manifest),
        (package.info.entrypoint.as_str(), &package.script),
    ] {
        let mut file = File::create(stage.path().join(name))?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    sync_directory(stage.path())?;
    fs::rename(stage.path(), &target)
        .context("cannot atomically publish complete module version")?;
    sync_directory(&versions)?;
    sync_directory(root)?;
    Ok(())
}

pub(super) fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path; // Windows uses SQLite FULL sync and flushed files; portable directory fsync is unavailable.
    Ok(())
}
