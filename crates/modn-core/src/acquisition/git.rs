//! Fetch one pinned Git commit without checkout and pack its tree.
//!
//! The commit hash is the integrity pin. Integration tests use a local bare
//! repository through a test-only transport override; release builds accept
//! only GitHub HTTPS remotes.
use super::{file_sha256, ArchiveCache, CacheUse, Origin};
use crate::git::{self, GitEnvironment};
use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use zip::write::SimpleFileOptions;

const MAX_ENTRIES: usize = 20_000;
const MAX_UNPACKED: u64 = 64 * 1024 * 1024 * 1024;

/// A package source pinned to one full lowercase Git commit on GitHub.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct GitSource {
    /// Repository in `owner/name` form.
    pub repository: String,
    /// Full 40-character lowercase commit hash. Branches and tags are refused.
    pub commit: String,
}

/// Verified Git archive cache key. The commit names the source revision.
#[expect(missing_docs, reason = "Field and method names state their meaning")]
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct GitCacheKey<'a> {
    pub game_id: &'a str,
    pub mod_id: &'a str,
    pub version: &'a str,
    pub commit: &'a str,
}

impl<'a> GitCacheKey<'a> {
    /// Name the package release and pinned commit of one cache entry.
    #[must_use]
    pub fn new(game_id: &'a str, mod_id: &'a str, version: &'a str, commit: &'a str) -> Self {
        Self {
            game_id,
            mod_id,
            version,
            commit,
        }
    }
}

/// One fetched archive in the cache, with the digest the pipeline records.
#[expect(missing_docs, reason = "Field names state their meaning")]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct GitAcquired {
    pub path: PathBuf,
    pub sha256: String,
    pub origin: Origin,
}

#[derive(Debug)]
pub(crate) struct PackedArchive {
    path: PathBuf,
    sha256: String,
}

struct TreeEntry {
    path: String,
    object: String,
    executable: bool,
}

fn valid_commit(commit: &str) -> bool {
    commit.len() == 40
        && commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Test-only repository base override. Release builds always use GitHub.
#[cfg(any(test, feature = "test-instruction-budget"))]
fn remote_base() -> Option<String> {
    std::env::var("MODN_TEST_GIT_REMOTE_BASE")
        .ok()
        .filter(|base| !base.is_empty())
}

#[cfg(not(any(test, feature = "test-instruction-budget")))]
fn remote_base() -> Option<String> {
    None
}

fn segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

impl GitSource {
    /// Construct a source. Validation happens before any Git call.
    #[must_use]
    pub fn new(repository: String, commit: String) -> Self {
        Self { repository, commit }
    }

    /// Validate the source and return the remote URL.
    ///
    /// Test builds may map the GitHub base to a local `file` URL. Release
    /// builds never read that override and always return GitHub HTTPS.
    pub fn remote_url(&self) -> Result<String> {
        let parts: Vec<&str> = self.repository.split('/').collect();
        if parts.len() != 2 || !parts.iter().all(|part| segment(part)) {
            bail!("invalid Git repository; expected owner/project");
        }
        if !valid_commit(&self.commit) {
            bail!("Git source commit must be exactly 40 lowercase hex characters");
        }
        let base = remote_base().unwrap_or_else(|| "https://github.com".to_owned());
        Ok(format!("{base}/{}.git", self.repository))
    }

    fn test_transport(&self, url: &str) -> bool {
        #[cfg(any(test, feature = "test-instruction-budget"))]
        {
            let _ = self;
            url.starts_with("file://")
        }
        #[cfg(not(any(test, feature = "test-instruction-budget")))]
        {
            let _ = url;
            false
        }
    }

    /// Fetch, verify and pack the pinned commit into a deterministic ZIP in
    /// `staging`. The caller owns `staging` and removes it afterwards.
    pub(crate) fn pack_url(&self, url: &str, staging: &Path) -> Result<PackedArchive> {
        let allow_file = self.test_transport(url);
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
            anyhow!(
                "GIT_SOURCE_FETCH_FAILED: cannot initialize the private Git repository: {error:#}"
            )
        })?;
        git::run(
            &environment,
            &[
                "fetch",
                "--depth",
                "1",
                "--no-tags",
                "--",
                url,
                &self.commit,
            ],
        )
        .map_err(|error| {
            anyhow!(
                "GIT_SOURCE_FETCH_FAILED: fetching {url} commit {} failed: {error:#}",
                self.commit
            )
        })?;
        let pinned = format!("{}^{{commit}}", self.commit);
        let resolved =
            git::run(&environment, &["rev-parse", "--verify", &pinned]).map_err(|error| {
                anyhow!(
                    "GIT_SOURCE_COMMIT_MISMATCH: commit {} is not present after fetch: {error:#}",
                    self.commit
                )
            })?;
        if resolved != self.commit {
            bail!(
                "GIT_SOURCE_COMMIT_MISMATCH: requested {} but Git resolved {resolved}",
                self.commit
            );
        }
        let tree = git::run_bytes(
            &environment,
            &["ls-tree", "-r", "-z", "--full-tree", &self.commit],
        )
        .map_err(|error| {
            anyhow!("GIT_SOURCE_FETCH_FAILED: cannot read the pinned tree: {error:#}")
        })?;
        let entries = parse_tree(&tree)?;
        let archive = staging.join("archive.zip");
        let sha256 = write_archive(&repository, entries, &archive)?;
        Ok(PackedArchive {
            path: archive,
            sha256,
        })
    }

    /// Return a verified cached archive or fetch and pack the commit.
    pub fn acquire(
        &self,
        cache: &ArchiveCache,
        key: &GitCacheKey<'_>,
        cache_use: CacheUse,
    ) -> Result<GitAcquired> {
        let url = self.remote_url()?;
        self.acquire_url(&url, cache, key, cache_use)
    }

    fn acquire_url(
        &self,
        url: &str,
        cache: &ArchiveCache,
        key: &GitCacheKey<'_>,
        cache_use: CacheUse,
    ) -> Result<GitAcquired> {
        if key.commit != self.commit {
            bail!("Git source cache key does not name the pinned commit");
        }
        if cache_use == CacheUse::Reuse {
            if let Some((path, sha256)) = cache.git_cached(key)? {
                return Ok(GitAcquired {
                    path,
                    sha256,
                    origin: Origin::Cache,
                });
            }
        }
        fs::create_dir_all(cache.root())
            .with_context(|| format!("cannot create cache directory {}", cache.root().display()))?;
        let staging = tempfile::Builder::new()
            .prefix("git-source-")
            .tempdir_in(cache.root())?;
        let packed = self.pack_url(url, staging.path())?;
        let path = cache.git_path(key, &packed.sha256);
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("cannot replace cache entry {}", path.display()))?;
        }
        fs::rename(&packed.path, &path)
            .with_context(|| format!("cannot store cache entry {}", path.display()))?;
        Ok(GitAcquired {
            path,
            sha256: packed.sha256,
            origin: Origin::Download,
        })
    }
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
            .last_modified_time(time)
            .unix_permissions(if entry.executable { 0o755 } else { 0o644 });
        zip.start_file(&entry.path, options)?;
        let mut remaining = size;
        while remaining > 0 {
            let length = remaining.min(64 * 1024) as usize;
            let mut buffer = vec![0; length];
            reader.read_exact(&mut buffer)?;
            zip.write_all(&buffer)?;
            remaining -= length as u64;
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
    if finished.metadata()?.len() > super::MAX_DOWNLOAD_BYTES {
        bail!("GIT_SOURCE_UNSUPPORTED_ENTRY: packed archive exceeds 4 GiB");
    }
    file_sha256(archive)
}

impl ArchiveCache {
    fn git_name(&self, key: &GitCacheKey<'_>, sha256: &str) -> String {
        format!(
            "{}#{}#{}#git-{}-{sha256}.zip",
            super::component(key.game_id),
            super::component(key.mod_id),
            super::component(key.version),
            key.commit
        )
    }

    /// Cache path for one commit and archive digest.
    #[must_use]
    pub fn git_path(&self, key: &GitCacheKey<'_>, sha256: &str) -> PathBuf {
        self.root.join(self.git_name(key, sha256))
    }

    /// Return a cached Git archive whose bytes match its recorded digest.
    ///
    /// A corrupt entry is never returned; the next fetch replaces it.
    pub fn git_cached(&self, key: &GitCacheKey<'_>) -> Result<Option<(PathBuf, String)>> {
        let directory = self.entries(None)?;
        let prefix = format!(
            "{}#{}#{}#git-{}-",
            super::component(key.game_id),
            super::component(key.mod_id),
            super::component(key.version),
            key.commit
        );
        for entry in directory {
            let name = entry
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let Some(rest) = name.strip_prefix(&prefix) else {
                continue;
            };
            let Some(sha256) = rest.strip_suffix(".zip") else {
                continue;
            };
            if sha256.len() != 64
                || !sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                continue;
            }
            if entry.bytes > super::MAX_DOWNLOAD_BYTES {
                continue;
            }
            if file_sha256(&entry.path)? == sha256 {
                return Ok(Some((entry.path, sha256.to_owned())));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests;
