//! Narrow, read-only Luau host services scoped to operator-approved directories.
use crate::archive;
use anyhow::{bail, Context, Result};
use mlua::{Lua, Table};
use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

const MAX_ROOTS: usize = 8;
const MAX_RELATIVE_BYTES: usize = 256;
const MAX_DEPTH: usize = 16;
const MAX_CALLS: u32 = 128;
const MAX_SCAN_CALLS: u32 = 20_128;
const MAX_TOTAL_DIRECTORY_ENTRIES: usize = 20_000;

#[derive(Clone, Copy)]
pub(super) enum ReadProfile {
    Standard,
    Scan,
}
const MAX_TOTAL_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ONE_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 256;

/// Operator-approved directories, not game installations authenticated by Rust.
/// A game module decides whether a directory belongs to its game using metadata.
pub(super) struct AllowedRoots(Vec<PathBuf>);

impl AllowedRoots {
    pub(super) fn validate(requested: &[PathBuf]) -> Result<Self> {
        if requested.len() > MAX_ROOTS {
            bail!("too many authorized roots; maximum is {MAX_ROOTS}");
        }
        let mut roots = Vec::new();
        for path in requested {
            let metadata = fs::symlink_metadata(path)
                .with_context(|| format!("cannot inspect authorized root {}", path.display()))?;
            if !metadata.file_type().is_dir() {
                bail!("authorized root must be a real directory, not a link");
            }
            let canonical = path.canonicalize()?;
            if roots.contains(&canonical) {
                bail!("duplicate authorized root");
            }
            roots.push(canonical);
        }
        Ok(Self(roots))
    }
}

fn relative_components(value: &str) -> Result<Vec<&str>> {
    if value.is_empty()
        || value.len() > MAX_RELATIVE_BYTES
        || value.contains('\\')
        || value.contains(':')
        || value.contains('\0')
    {
        bail!("unsafe relative path");
    }
    let parts: Vec<_> = value.split('/').collect();
    if parts.len() > MAX_DEPTH
        || parts.iter().any(|part| {
            part.is_empty()
                || *part == "."
                || *part == ".."
                || part.ends_with([' ', '.'])
                || part.chars().any(char::is_control)
        })
    {
        bail!("unsafe relative path");
    }
    let path = Path::new(value);
    if path
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!("unsafe relative path");
    }
    Ok(parts)
}

fn safe_target(root: &Path, relative: &str) -> Result<(PathBuf, fs::Metadata)> {
    let parts = relative_components(relative)?;
    let mut current = root.to_path_buf();
    for (index, part) in parts.iter().enumerate() {
        current.push(part);
        let metadata = fs::symlink_metadata(&current)
            .with_context(|| format!("cannot inspect requested path {relative:?}"))?;
        if metadata.file_type().is_symlink()
            || (!metadata.file_type().is_file() && !metadata.file_type().is_dir())
        {
            bail!("linked or special paths are not allowed");
        }
        if index + 1 < parts.len() && !metadata.is_dir() {
            bail!("non-directory component in requested path");
        }
        let canonical = current.canonicalize()?;
        if !canonical.starts_with(root) {
            bail!("requested path escapes authorized root");
        }
    }
    let metadata = fs::symlink_metadata(&current)?;
    if metadata.file_type().is_symlink()
        || (!metadata.file_type().is_file() && !metadata.file_type().is_dir())
    {
        bail!("linked or special paths are not allowed");
    }
    if !current.canonicalize()?.starts_with(root) {
        bail!("requested path escapes authorized root");
    }
    Ok((current, metadata))
}

struct Budget {
    calls: Cell<u32>,
    bytes: Cell<u64>,
    directory_entries: Cell<usize>,
    call_limit: u32,
}
impl Budget {
    fn new(profile: ReadProfile) -> Self {
        Self {
            calls: Cell::new(0),
            bytes: Cell::new(0),
            directory_entries: Cell::new(0),
            call_limit: match profile {
                ReadProfile::Standard => MAX_CALLS,
                ReadProfile::Scan => MAX_SCAN_CALLS,
            },
        }
    }

    fn operation(&self) -> Result<()> {
        let next = self.calls.get().saturating_add(1);
        if next > self.call_limit {
            bail!("read-only capability call budget exceeded");
        }
        self.calls.set(next);
        Ok(())
    }

    fn charge(&self, bytes: u64) -> Result<()> {
        if bytes > MAX_ONE_FILE_BYTES {
            bail!("file exceeds per-call read limit");
        }
        let total = self.bytes.get().saturating_add(bytes);
        if total > MAX_TOTAL_FILE_BYTES {
            bail!("read-only capability byte budget exceeded");
        }
        self.bytes.set(total);
        Ok(())
    }
}

fn selected_root(roots: &AllowedRoots, id: u32) -> Result<&Path> {
    if id == 0 {
        bail!("invalid authorized root identifier");
    }
    roots
        .0
        .get((id - 1) as usize)
        .map(PathBuf::as_path)
        .context("unknown authorized root identifier")
}

fn authorized_path(
    roots: &AllowedRoots,
    budget: &Budget,
    root_id: u32,
    relative: &str,
) -> Result<(PathBuf, fs::Metadata)> {
    budget.operation()?;
    let root = selected_root(roots, root_id)?;
    safe_target(root, relative)
}

fn regular_file(metadata: &fs::Metadata) -> Result<()> {
    if !metadata.file_type().is_file() {
        bail!("capability requires a regular file");
    }
    Ok(())
}

fn file_digest(path: &Path, expected_bytes: u64) -> Result<String> {
    let mut hasher = Sha256::new();
    let file = File::open(path)?;
    let mut input = file.take(expected_bytes + 1);
    let mut buffer = [0u8; 32 * 1024];
    let mut total = 0u64;
    loop {
        let bytes = input.read(&mut buffer)?;
        if bytes == 0 {
            break;
        }
        total += bytes as u64;
        if total > expected_bytes {
            bail!("file changed size during hashing");
        }
        hasher.update(&buffer[..bytes]);
    }
    if total != expected_bytes {
        bail!("file changed size during hashing");
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn metadata_service(lua: &Lua, roots: Rc<AllowedRoots>, budget: Rc<Budget>) -> mlua::Result<Table> {
    let service = lua.create_table()?;
    service.set(
        "metadata",
        lua.create_function(move |lua, (id, relative): (u32, String)| {
            let (_path, metadata) =
                authorized_path(&roots, &budget, id, &relative).map_err(mlua::Error::external)?;
            let answer = lua.create_table()?;
            answer.set(
                "kind",
                if metadata.is_dir() {
                    "directory"
                } else {
                    "file"
                },
            )?;
            answer.set("bytes", if metadata.is_dir() { 0 } else { metadata.len() })?;
            // No absolute path or operating-system handle is returned.
            answer.set_readonly(true);
            Ok(answer)
        })?,
    )?;
    service.set_readonly(true);
    Ok(service)
}

fn hashing_service(lua: &Lua, roots: Rc<AllowedRoots>, budget: Rc<Budget>) -> mlua::Result<Table> {
    let service = lua.create_table()?;
    service.set(
        "sha256",
        lua.create_function(move |_, (id, relative): (u32, String)| {
            let (path, metadata) =
                authorized_path(&roots, &budget, id, &relative).map_err(mlua::Error::external)?;
            regular_file(&metadata).map_err(mlua::Error::external)?;
            budget
                .charge(metadata.len())
                .map_err(mlua::Error::external)?;
            file_digest(&path, metadata.len()).map_err(mlua::Error::external)
        })?,
    )?;
    service.set_readonly(true);
    Ok(service)
}

fn archive_service(lua: &Lua, roots: Rc<AllowedRoots>, budget: Rc<Budget>) -> mlua::Result<Table> {
    let service = lua.create_table()?;
    service.set(
        "inspect",
        lua.create_function(move |lua, (id, relative): (u32, String)| {
            let (path, metadata) =
                authorized_path(&roots, &budget, id, &relative).map_err(mlua::Error::external)?;
            regular_file(&metadata).map_err(mlua::Error::external)?;
            // 7z encoded headers can expand unboundedly inside the native parser,
            // before its entry-count validation. Do not expose that path to Luau
            // until decoded-header allocations are demonstrably bounded.
            let extension = path.extension().and_then(|value| value.to_str()).unwrap_or("");
            if !extension.eq_ignore_ascii_case("zip") && !extension.eq_ignore_ascii_case("jar") {
                return Err(mlua::Error::RuntimeError(
                    "Luau archive inspection supports ZIP/JAR only; 7z decoded headers are not bounded"
                        .into(),
                ));
            }
            budget
                .charge(metadata.len())
                .map_err(mlua::Error::external)?;
            let report = archive::inspect(&path).map_err(mlua::Error::external)?;
            if report.files.len() > MAX_ARCHIVE_ENTRIES {
                return Err(mlua::Error::RuntimeError(
                    "archive exceeds Luau result entry limit".into(),
                ));
            }
            let answer = lua.create_table()?;
            answer.set("format", report.format)?;
            answer.set("sha256", report.sha256)?;
            answer.set("total_uncompressed_bytes", report.total_uncompressed_bytes)?;
            answer.set("metadata_only", true)?;
            let files = lua.create_table()?;
            for (index, entry) in report.files.iter().enumerate() {
                let item = lua.create_table()?;
                item.set("entry", entry.entry.as_str())?;
                item.set("bytes", entry.bytes)?;
                item.set_readonly(true);
                files.set(index + 1, item)?;
            }
            files.set_readonly(true);
            answer.set("files", files)?;
            answer.set_readonly(true);
            Ok(answer)
        })?,
    )?;
    service.set_readonly(true);
    Ok(service)
}

const MAX_DIRECTORY_ENTRIES: usize = 2_048;
const MAX_TEXT_BYTES: u64 = 64 * 1024;

fn directory_service(
    lua: &Lua,
    roots: Rc<AllowedRoots>,
    budget: Rc<Budget>,
) -> mlua::Result<Table> {
    let service = lua.create_table()?;
    let read_roots = Rc::clone(&roots);
    let read_budget = Rc::clone(&budget);
    service.set(
        "list",
        lua.create_function(
            move |lua, (id, relative, max_items): (u32, String, Option<usize>)| {
                let max_items = max_items
                    .unwrap_or(MAX_DIRECTORY_ENTRIES)
                    .min(MAX_DIRECTORY_ENTRIES);
                let (path, metadata) = authorized_path(&read_roots, &read_budget, id, &relative)
                    .map_err(mlua::Error::external)?;
                if !metadata.is_dir() {
                    return Err(mlua::Error::RuntimeError(
                        "listing requires a directory".into(),
                    ));
                }
                let root = selected_root(&read_roots, id).map_err(mlua::Error::external)?;
                let mut names = Vec::new();
                let mut incomplete = false;
                for entry in fs::read_dir(path).map_err(mlua::Error::external)? {
                    // Cap the native enumeration before allocating Lua tables.
                    if names.len() >= max_items {
                        incomplete = true;
                        break;
                    }
                    let next = read_budget.directory_entries.get().saturating_add(1);
                    if next > MAX_TOTAL_DIRECTORY_ENTRIES {
                        incomplete = true;
                        break;
                    }
                    read_budget.directory_entries.set(next);
                    let entry = entry.map_err(mlua::Error::external)?;
                    let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                        incomplete = true;
                        continue;
                    };
                    let relative_entry = format!("{relative}/{name}");
                    if relative_components(&relative_entry).is_err() {
                        incomplete = true;
                        continue;
                    }
                    let Ok((_, metadata)) = safe_target(root, &relative_entry) else {
                        continue;
                    };
                    names.push((
                        name,
                        if metadata.is_dir() {
                            "directory"
                        } else {
                            "file"
                        },
                    ));
                }
                names.sort_by(|a, b| a.0.cmp(&b.0));
                let items = lua.create_table()?;
                for (index, (name, kind)) in names.iter().enumerate() {
                    let item = lua.create_table()?;
                    item.set("name", name.as_str())?;
                    item.set("kind", *kind)?;
                    item.set_readonly(true);
                    items.set(index + 1, item)?;
                }
                items.set_readonly(true);
                let response = lua.create_table()?;
                response.set("items", items)?;
                response.set("incomplete", incomplete)?;
                response.set_readonly(true);
                Ok(response)
            },
        )?,
    )?;
    service.set(
        "read_text",
        lua.create_function(move |_, (id, relative): (u32, String)| {
            let (path, metadata) =
                authorized_path(&roots, &budget, id, &relative).map_err(mlua::Error::external)?;
            regular_file(&metadata).map_err(mlua::Error::external)?;
            if metadata.len() > MAX_TEXT_BYTES {
                return Err(mlua::Error::RuntimeError(
                    "text file exceeds 64 KiB limit".into(),
                ));
            }
            budget
                .charge(metadata.len())
                .map_err(mlua::Error::external)?;
            let mut data = Vec::new();
            File::open(path)
                .map_err(mlua::Error::external)?
                .take(metadata.len() + 1)
                .read_to_end(&mut data)
                .map_err(mlua::Error::external)?;
            if data.len() as u64 != metadata.len() {
                return Err(mlua::Error::RuntimeError(
                    "text file changed during read".into(),
                ));
            }
            String::from_utf8(data).map_err(mlua::Error::external)
        })?,
    )?;
    service.set_readonly(true);
    Ok(service)
}

/// Add only read-only host services to the existing sandbox; never pass raw paths.
pub(super) fn register(
    lua: &Lua,
    host: &Table,
    roots: AllowedRoots,
    profile: ReadProfile,
) -> mlua::Result<()> {
    let roots = Rc::new(roots);
    let budget = Rc::new(Budget::new(profile));
    let discovery = lua.create_table()?;
    let allowed = Rc::clone(&roots);
    let limit = Rc::clone(&budget);
    discovery.set(
        "roots",
        lua.create_function(move |lua, ()| {
            limit.operation().map_err(mlua::Error::external)?;
            let result = lua.create_table()?;
            for index in 0..allowed.0.len() {
                let item = lua.create_table()?;
                item.set("id", index + 1)?;
                item.set_readonly(true);
                result.set(index + 1, item)?;
            }
            result.set_readonly(true);
            Ok(result)
        })?,
    )?;
    discovery.set_readonly(true);
    host.set("discovery", discovery)?;
    host.set(
        "fs",
        metadata_service(lua, Rc::clone(&roots), Rc::clone(&budget))?,
    )?;
    host.set(
        "hash",
        hashing_service(lua, Rc::clone(&roots), Rc::clone(&budget))?,
    )?;
    host.set(
        "archive",
        archive_service(lua, Rc::clone(&roots), Rc::clone(&budget))?,
    )?;
    host.set("directory", directory_service(lua, roots, budget)?)?;
    Ok(())
}
