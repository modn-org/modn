//! Fetch one pinned Git commit and pack its tree into a deterministic ZIP.

use super::super::digest::file_blake3;
use super::super::{FetchError, CONNECT_TIMEOUT, READ_INACTIVITY_TIMEOUT};
use crate::git::{self, GitEnvironment};
use anyhow::{anyhow, bail, Context, Result};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use zip::write::SimpleFileOptions;

mod process;
use process::FetchProcess;

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

/// How often the fetch loop checks whether its child exited or progressed.
const FETCH_POLL: Duration = Duration::from_millis(25);

/// How long the timeout path waits for the ended child and its reader.
const FETCH_KILL_GRACE: Duration = Duration::from_secs(2);

/// How long one Git fetch may make no progress before it fails.
///
/// The value matches the connect and idle timeouts that `docs/sources.md`
/// documents for every source. A fetch that keeps writing progress output may
/// run longer; a silent or stalled transfer fails. A test build may shorten
/// the limit through `MODN_TEST_GIT_FETCH_IDLE_MS`.
fn fetch_inactivity_limit() -> Duration {
    #[cfg(any(test, feature = "test-instruction-budget"))]
    if let Some(millis) = std::env::var("MODN_TEST_GIT_FETCH_IDLE_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
    {
        return Duration::from_millis(millis.max(1));
    }
    CONNECT_TIMEOUT + READ_INACTIVITY_TIMEOUT
}

/// A test-only replacement for the Git program; production uses Git itself.
#[cfg(any(test, feature = "test-instruction-budget"))]
fn fetch_program() -> Option<OsString> {
    std::env::var_os("MODN_TEST_GIT_FETCH_PROGRAM")
}

#[cfg(not(any(test, feature = "test-instruction-budget")))]
fn fetch_program() -> Option<OsString> {
    None
}

/// Fetch `commit` from `url`; fail only when the fetch stops making progress.
///
/// Git runs with `--progress`, so its stderr carries progress output while the
/// transfer advances. The fetch fails after the inactivity limit passes with no
/// new stderr output, which covers a stalled transfer and a remote that never
/// answers. A transfer that Git itself stops below its low-speed limit keeps
/// the same [`FetchError::Timeout`] class.
fn fetch_commit(environment: &GitEnvironment<'_>, url: &str, commit: &str) -> Result<()> {
    fetch_with(
        environment,
        url,
        commit,
        fetch_inactivity_limit(),
        fetch_program().as_deref(),
    )
}

/// Fetch with an explicit inactivity limit and program, for tests.
fn fetch_with(
    environment: &GitEnvironment<'_>,
    url: &str,
    commit: &str,
    limit: Duration,
    program: Option<&OsStr>,
) -> Result<()> {
    let mut fetch = FetchProcess::spawn(environment, url, commit, program)?;
    let mut last_progress = Instant::now();
    let mut seen = 0;
    let stalled = loop {
        if fetch.try_wait()?.is_some() {
            break false;
        }
        let progress = fetch.progress();
        if progress != seen {
            seen = progress;
            last_progress = Instant::now();
        }
        if last_progress.elapsed() >= limit {
            break true;
        }
        std::thread::sleep(FETCH_POLL);
    };
    if stalled {
        // Git and every descendant end before the caller removes the staging
        // directory or moves on to the next source.
        fetch.terminate(FETCH_KILL_GRACE);
        let _ = fetch.stderr(FETCH_KILL_GRACE);
        return Err(anyhow::Error::new(FetchError::Timeout).context(format!(
            "GIT_SOURCE_FETCH_FAILED: fetching {url} commit {commit} made no progress for {} ms",
            limit.as_millis()
        )));
    }
    let status = fetch.wait()?;
    let stderr = fetch.stderr(FETCH_KILL_GRACE);
    if !status.success() {
        if low_speed_abort(&stderr) {
            return Err(anyhow::Error::new(FetchError::Timeout).context(format!(
                "GIT_SOURCE_FETCH_FAILED: fetching {url} commit {commit} stopped below the Git low-speed limit: {}",
                stderr.trim()
            )));
        }
        bail!(
            "GIT_SOURCE_FETCH_FAILED: fetching {url} commit {commit} failed: {}",
            stderr.trim()
        );
    }
    Ok(())
}

/// Whether Git's stderr names the transfer abort below its low-speed limit.
///
/// `git fetch` runs with `http.lowSpeedLimit` and `http.lowSpeedTime`, so
/// libcurl ends a stalled HTTP transfer with its operation-timeout message
/// before the inactivity limit elapses. The ordered fallback treats that abort
/// like the deadline: both are a `timeout` attempt.
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
    fetch_commit(&environment, url, commit)?;
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
