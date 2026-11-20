//! Fetch one pinned Git commit and pack its tree into a deterministic ZIP.

use super::super::digest::file_blake3;
use super::super::FetchError;
use crate::git::{self, GitEnvironment};
use anyhow::{anyhow, bail, Context, Result};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use zip::write::SimpleFileOptions;

#[cfg(test)]
mod tests;

const MAX_ENTRIES: usize = 20_000;
const MAX_UNPACKED: u64 = 64 * 1024 * 1024 * 1024;

/// A packed Git tree in `staging`, with the BLAKE3 digest of the ZIP bytes.
#[derive(Debug)]
pub(crate) struct PackedArchive {
    pub(crate) path: PathBuf,
    pub(crate) blake3: String,
}

struct TreeEntry {
    path: String,
    object: String,
    executable: bool,
}

/// Fetch `commit` from `url`; Git bounds a stalled HTTP(S) transfer itself.
///
/// The shared Git command passes `http.lowSpeedLimit` and `http.lowSpeedTime`,
/// so a transfer that stays below one byte per second for the documented
/// inactivity limit ends with Git's own abort. That abort keeps the
/// [`FetchError::Timeout`] class, which lets ordered acquisition try the next
/// source. Git prints progress while it receives data, so a transfer that
/// keeps receiving data may run longer than the limit.
fn fetch_with(environment: &GitEnvironment<'_>, url: &str, commit: &str) -> Result<()> {
    fetch_with_command(git::command(environment), url, commit)
}

/// Run one prepared fetch command and classify its failure.
fn fetch_with_command(mut command: Command, url: &str, commit: &str) -> Result<()> {
    command
        .args([
            "fetch",
            "--progress",
            "--depth",
            "1",
            "--no-tags",
            "--",
            url,
            commit,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let output = command
        .output()
        .context("GIT_SOURCE_FETCH_FAILED: cannot run Git; install Git and ensure it is on PATH")?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if low_speed_abort(&stderr) {
        return Err(anyhow::Error::new(FetchError::Timeout).context(format!(
            "GIT_SOURCE_FETCH_FAILED: fetching {url} commit {commit} stopped below the Git low-speed limit: {}",
            stderr.trim()
        )));
    }
    bail!(
        "GIT_SOURCE_FETCH_FAILED: fetching {url} commit {commit} failed: {}",
        stderr.trim()
    )
}

/// Clone one Git catalog revision under the shared low-speed limits.
///
/// The clone keeps every hostile-input protection of the catalog module: the
/// empty hooks directory and the allow-listed transports of the shared Git
/// command. Git ends a stalled HTTP(S) transfer itself; the catalog sync
/// records that failure as `timed out` and tries the next mirror.
pub(super) fn clone_catalog(destination: &Path, remote: &str) -> Result<()> {
    let hooks = git::empty_hooks()?;
    let environment = GitEnvironment {
        directory: destination,
        hooks: hooks.path(),
        allow_file: true,
    };
    let output = git::command(&environment)
        .args([
            "clone",
            "--progress",
            "--no-local",
            "--no-checkout",
            "--",
            remote,
            "catalog",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context(
            "GIT_CATALOG_FETCH_FAILED: cannot run Git; install Git and ensure it is on PATH",
        )?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if low_speed_abort(&stderr) {
        return Err(anyhow::Error::new(FetchError::Timeout).context(format!(
            "GIT_CATALOG_FETCH_FAILED: cloning {remote} timed out below the Git low-speed limit: {}",
            stderr.trim()
        )));
    }
    bail!(
        "GIT_CATALOG_FETCH_FAILED: cloning {remote} failed: {}",
        stderr.trim()
    )
}

/// Whether Git's stderr names the transfer abort below its low-speed limit.
///
/// `git fetch` and `git clone` run with `http.lowSpeedLimit` and
/// `http.lowSpeedTime`, so libcurl ends a stalled HTTP or HTTPS transfer with
/// `Operation too slow. Less than 1 bytes/sec transferred the last N seconds`.
/// The ordered fallback treats that abort like the deadline: both are a
/// `timeout` attempt. File and SSH transports do not apply the limits.
fn low_speed_abort(stderr: &str) -> bool {
    let text = stderr.to_ascii_lowercase();
    text.contains("operation too slow") || text.contains("bytes/sec transferred")
}

/// Fetch `commit` from `url` and pack its tree into `staging/archive.zip`.
pub(super) fn pack_commit(
    url: &str,
    commit: &str,
    staging: &Path,
    allow_file: bool,
) -> Result<PackedArchive> {
    let hooks = git::empty_hooks()?;
    let environment = GitEnvironment {
        directory: staging,
        hooks: hooks.path(),
        allow_file,
    };
    let repository = staging.join("source.git");
    fs::create_dir_all(&repository)?;
    let environment = GitEnvironment {
        directory: &repository,
        ..environment
    };
    git::run(&environment, &["init", "--bare", "--quiet"]).map_err(|error| {
        anyhow!("GIT_SOURCE_FETCH_FAILED: cannot initialize the private Git repository: {error:#}")
    })?;
    fetch_with(&environment, url, commit)?;
    let pinned = format!("{commit}^{{commit}}");
    let resolved =
        git::run(&environment, &["rev-parse", "--verify", &pinned]).map_err(|error| {
            anyhow!(
                "GIT_SOURCE_COMMIT_MISMATCH: commit {commit} is not present after fetch: {error:#}"
            )
        })?;
    if resolved != commit {
        bail!("GIT_SOURCE_COMMIT_MISMATCH: requested {commit} but Git resolved {resolved}");
    }
    let tree = git::run_bytes(
        &environment,
        &["ls-tree", "-r", "-z", "--full-tree", commit],
    )
    .map_err(|error| anyhow!("GIT_SOURCE_FETCH_FAILED: cannot read the pinned tree: {error:#}"))?;
    let entries = parse_tree(&tree)?;
    let archive = staging.join("archive.zip");
    let blake3 = write_archive(&repository, entries, &archive)?;
    Ok(PackedArchive {
        path: archive,
        blake3,
    })
}

fn parse_tree(tree: &[u8]) -> Result<Vec<TreeEntry>> {
    let mut entries = Vec::new();
    for record in tree
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let Some(tab) = record.iter().position(|byte| *byte == b'\t') else {
            bail!("GIT_SOURCE_UNSUPPORTED_ENTRY: malformed Git tree entry");
        };
        let (metadata, path) = record.split_at(tab);
        let path = &path[1..];
        let metadata = std::str::from_utf8(metadata)
            .context("GIT_SOURCE_UNSUPPORTED_ENTRY: malformed Git tree metadata")?;
        let mut fields = metadata.split(' ');
        let (Some(mode), Some(kind), Some(object), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            bail!("GIT_SOURCE_UNSUPPORTED_ENTRY: malformed Git tree metadata");
        };
        if mode == "120000" {
            bail!("GIT_SOURCE_UNSUPPORTED_ENTRY: symbolic links are not supported");
        }
        if mode == "160000" || kind == "commit" {
            bail!("GIT_SOURCE_UNSUPPORTED_ENTRY: submodules are not supported");
        }
        if kind != "blob" || !matches!(mode, "100644" | "100755") {
            bail!("GIT_SOURCE_UNSUPPORTED_ENTRY: only regular files are supported");
        }
        if entries.len() >= MAX_ENTRIES {
            bail!("GIT_SOURCE_UNSUPPORTED_ENTRY: tree has more than {MAX_ENTRIES} entries");
        }
        let path = std::str::from_utf8(path)
            .context("GIT_SOURCE_UNSUPPORTED_ENTRY: file name is not UTF-8")?;
        let path = crate::archive::validated_name(path)
            .map_err(|error| anyhow!("GIT_SOURCE_UNSUPPORTED_ENTRY: {error:#}"))?;
        entries.push(TreeEntry {
            path,
            object: object.to_owned(),
            executable: mode == "100755",
        });
    }
    entries.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    let mut unique = std::collections::HashSet::new();
    if entries
        .iter()
        .any(|entry| !unique.insert(entry.path.to_lowercase()))
    {
        bail!("GIT_SOURCE_UNSUPPORTED_ENTRY: duplicate or case-colliding paths");
    }
    if entries.is_empty() {
        bail!("GIT_SOURCE_UNSUPPORTED_ENTRY: the pinned tree contains no files");
    }
    Ok(entries)
}

fn write_archive(repository: &Path, entries: Vec<TreeEntry>, archive: &Path) -> Result<String> {
    let hooks = git::empty_hooks()?;
    let environment = GitEnvironment {
        directory: repository,
        hooks: hooks.path(),
        allow_file: false,
    };
    let mut child = git::command(&environment)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("GIT_SOURCE_FETCH_FAILED: cannot run git cat-file")?;
    let mut input = child.stdin.take().context("git cat-file stdin")?;
    let objects: Vec<String> = entries.iter().map(|entry| entry.object.clone()).collect();
    let writer = std::thread::spawn(move || {
        for object in objects {
            if writeln!(input, "{object}").is_err() {
                return;
            }
        }
    });
    let output = child.stdout.take().context("git cat-file stdout")?;
    let mut reader = BufReader::new(output);
    let file =
        File::create(archive).with_context(|| format!("cannot create {}", archive.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let mut unpacked = 0u64;
    for entry in entries {
        let mut header = String::new();
        reader
            .read_line(&mut header)
            .context("GIT_SOURCE_FETCH_FAILED: git cat-file ended early")?;
        let mut fields = header.trim_end().split(' ');
        let (Some(_object), Some(kind), Some(size)) = (fields.next(), fields.next(), fields.next())
        else {
            bail!("GIT_SOURCE_FETCH_FAILED: unexpected git cat-file output");
        };
        if kind != "blob" {
            bail!("GIT_SOURCE_UNSUPPORTED_ENTRY: only regular files are supported");
        }
        let size: u64 = size
            .parse()
            .context("GIT_SOURCE_FETCH_FAILED: invalid blob size")?;
        unpacked = unpacked
            .checked_add(size)
            .context("GIT_SOURCE_UNSUPPORTED_ENTRY: tree size overflow")?;
        if unpacked > MAX_UNPACKED {
            bail!("GIT_SOURCE_UNSUPPORTED_ENTRY: tree exceeds 64 GiB uncompressed");
        }
        let time = zip::DateTime::from_date_and_time(1980, 1, 1, 0, 0, 0)
            .context("fixed ZIP timestamp is valid")?;
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(if entry.executable { 0o755 } else { 0o644 })
            .last_modified_time(time);
        zip.start_file(entry.path, options)?;
        let copied = std::io::copy(&mut reader.by_ref().take(size), &mut zip)?;
        if copied != size {
            bail!("GIT_SOURCE_FETCH_FAILED: git cat-file ended early");
        }
        let mut newline = [0u8; 1];
        reader.read_exact(&mut newline)?;
    }
    let finished = zip.finish()?;
    finished.sync_all()?;
    let _ = writer.join();
    let status = child.wait()?;
    if !status.success() {
        bail!(
            "GIT_SOURCE_FETCH_FAILED: git cat-file failed: {}",
            String::from_utf8_lossy(&{
                let mut stderr = Vec::new();
                if let Some(mut error) = child.stderr.take() {
                    let _ = error.read_to_end(&mut stderr);
                }
                stderr
            })
        );
    }
    if finished.metadata()?.len() > super::super::MAX_DOWNLOAD_BYTES {
        bail!("GIT_SOURCE_UNSUPPORTED_ENTRY: packed archive exceeds 4 GiB");
    }
    file_blake3(archive)
}
