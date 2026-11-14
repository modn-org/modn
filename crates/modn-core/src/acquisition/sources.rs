//! Ordered artifact-source fallback with the documented timeout and retry policy.
//!
//! [`ArchiveCache::acquire_from_sources`] tries the sources of a release in
//! list order. It moves to the next source after a timeout, a network failure,
//! an HTTP failure or a BLAKE3 mismatch. A mismatched or partial body is never
//! cached. When every source fails, one error names each source and why it
//! failed.

#![expect(missing_docs, reason = "Field and variant names state their meaning")]

use super::digest::copy_blake3;
use super::index::{ArchiveIndex, ArchivePin};
use super::{Acquired, ArchiveCache, CacheKey, CacheUse, Origin, MAX_DOWNLOAD_BYTES};
use crate::error::{coded, ErrorCode};
use crate::progress::{DownloadReader, OperationControl};
use anyhow::Result;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::time::Duration;

/// Connect timeout applied to one artifact source.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Read-inactivity timeout applied to one artifact source.
pub const READ_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(60);
/// Extra attempts for one source after a retryable failure.
pub const RETRIES_PER_SOURCE: u32 = 1;

/// Timeout and retry settings for acquiring one archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourcePolicy {
    connect_timeout: Duration,
    idle_timeout: Duration,
    retries: u32,
}

impl SourcePolicy {
    /// Use `connect_timeout`, `idle_timeout` and `retries` for every source.
    #[must_use]
    pub fn new(connect_timeout: Duration, idle_timeout: Duration, retries: u32) -> Self {
        Self {
            connect_timeout,
            idle_timeout,
            retries,
        }
    }

    #[must_use]
    pub fn connect_timeout(&self) -> Duration {
        self.connect_timeout
    }

    #[must_use]
    pub fn idle_timeout(&self) -> Duration {
        self.idle_timeout
    }

    #[must_use]
    pub fn retries(&self) -> u32 {
        self.retries
    }

    fn retryable(&self, failure: &SourceFailure) -> bool {
        if self.retries == 0 {
            return false;
        }
        match failure {
            SourceFailure::Timeout | SourceFailure::Network(_) => true,
            SourceFailure::Http(status) => (500..600).contains(status),
            _ => false,
        }
    }
}

impl Default for SourcePolicy {
    fn default() -> Self {
        Self::new(CONNECT_TIMEOUT, READ_INACTIVITY_TIMEOUT, RETRIES_PER_SOURCE)
    }
}

/// Failure of one transport call, before the body is verified.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FetchError {
    /// The connect or read-inactivity timeout elapsed.
    Timeout,
    /// The connection failed or the body could not be read.
    Network(String),
    /// The server answered with an HTTP status other than 200.
    Http(u16),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => f.write_str("timeout"),
            Self::Network(message) => write!(f, "network: {message}"),
            Self::Http(status) => write!(f, "http {status}"),
        }
    }
}

impl std::error::Error for FetchError {}

/// One response body and its stated length, when the transport knows it.
#[non_exhaustive]
pub struct SourceBody {
    pub reader: Box<dyn Read>,
    pub total: Option<u64>,
}

impl SourceBody {
    /// A body read from `reader`, whose length the source states as `total`.
    #[must_use]
    pub fn new(reader: Box<dyn Read>, total: Option<u64>) -> Self {
        Self { reader, total }
    }
}

/// Network transport used by the source fallback loop.
pub trait SourceTransport {
    /// Open `url` under `policy`.
    fn open(&self, url: &str, policy: &SourcePolicy) -> Result<SourceBody, FetchError>;
}

/// What Modn may do with one entry of a release's ordered source list.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SourceTarget {
    /// Fetch this HTTPS URL.
    Fetch(String),
    /// A manual download Modn never starts automatically.
    Manual(String),
    /// A source this command does not acquire automatically.
    Refused(String),
}

/// One ordered source, with the label its attempt output uses.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SourceCandidate {
    pub label: String,
    pub target: SourceTarget,
}

impl SourceCandidate {
    /// A fetchable source; `url` is also its label.
    #[must_use]
    pub fn fetch(url: impl Into<String>) -> Self {
        let url = url.into();
        Self {
            label: url.clone(),
            target: SourceTarget::Fetch(url),
        }
    }

    /// A source the caller names but Modn cannot fetch automatically.
    #[must_use]
    pub fn refused(label: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            target: SourceTarget::Refused(reason.into()),
        }
    }

    /// A `manual` source.
    #[must_use]
    pub fn manual(filename: impl Into<String>) -> Self {
        let filename = filename.into();
        Self {
            label: format!("manual: {filename}"),
            target: SourceTarget::Manual(filename),
        }
    }
}

/// Why one attempt did not supply a verified archive.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SourceFailure {
    /// The connect or read-inactivity timeout elapsed.
    Timeout,
    /// The connection failed or the body stopped.
    Network(String),
    /// The server answered with another HTTP status.
    Http(u16),
    /// The bytes did not match the pinned digest.
    DigestMismatch { expected: String, actual: String },
    /// The source was not attempted, and why.
    Refused(String),
    /// A manual download is required.
    Manual,
}

impl fmt::Display for SourceFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => f.write_str("timeout"),
            Self::Network(message) => write!(f, "network: {message}"),
            Self::Http(status) => write!(f, "http {status}"),
            Self::DigestMismatch { expected, actual } => {
                write!(f, "digest mismatch (expected {expected}, actual {actual})")
            }
            Self::Refused(reason) => write!(f, "refused: {reason}"),
            Self::Manual => f.write_str("manual"),
        }
    }
}

impl From<FetchError> for SourceFailure {
    fn from(error: FetchError) -> Self {
        match error {
            FetchError::Timeout => Self::Timeout,
            FetchError::Network(message) => Self::Network(message),
            FetchError::Http(status) => Self::Http(status),
        }
    }
}

/// One attempt against one source.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SourceAttempt {
    pub source: String,
    pub attempt: u32,
    pub failure: Option<SourceFailure>,
}

impl SourceAttempt {
    /// Whether the attempt supplied verified bytes.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.failure.is_none()
    }

    /// A one-line outcome for text and JSON output.
    #[must_use]
    pub fn describe(&self) -> String {
        match &self.failure {
            Some(failure) => format!("attempt {}: {failure}", self.attempt),
            None => "ok".to_owned(),
        }
    }
}

/// A verified archive and every attempt the loop made.
#[derive(Debug)]
#[non_exhaustive]
pub struct SourcesAcquired {
    pub acquired: Acquired,
    pub attempts: Vec<SourceAttempt>,
}

/// Every source failed; the message names each one and why.
#[derive(Debug)]
pub struct SourcesError {
    reference: String,
    attempts: Vec<SourceAttempt>,
}

impl fmt::Display for SourcesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "no source supplied the archive for {}; every source that was tried failed:",
            self.reference
        )?;
        let mut index = 0;
        while index < self.attempts.len() {
            let source = &self.attempts[index].source;
            let mut end = index + 1;
            while end < self.attempts.len() && &self.attempts[end].source == source {
                end += 1;
            }
            let count = end - index;
            let last = &self.attempts[end - 1];
            let reason = last
                .failure
                .as_ref()
                .map_or_else(|| "unknown".to_owned(), ToString::to_string);
            if count == 1 {
                writeln!(f, "  {source}: {reason}")?;
            } else {
                writeln!(f, "  {source}: {reason} ({count} attempts)")?;
            }
            index = end;
        }
        Ok(())
    }
}

impl std::error::Error for SourcesError {}

impl ArchiveCache {
    /// Return a verified archive, trying `sources` in list order.
    ///
    /// A verified cache hit for the pinned digest is used before any transport
    /// call. Each other source is attempted at most `policy.retries() + 1`
    /// times after a retryable failure. A digest mismatch is final for that
    /// source. A failed attempt leaves no partial file, index record or cache
    /// entry.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::Cancelled`] when `control` is cancelled, and
    /// [`ErrorCode::DownloadFailed`] when every source fails.
    #[expect(
        clippy::too_many_arguments,
        reason = "the loop keeps the cache, policy, transport, package and cancellation explicit"
    )]
    pub fn acquire_from_sources(
        &self,
        key: &CacheKey<'_>,
        cache_use: CacheUse,
        sources: &[SourceCandidate],
        transport: &dyn SourceTransport,
        policy: SourcePolicy,
        package: Option<String>,
        control: &OperationControl,
    ) -> Result<SourcesAcquired> {
        if cache_use == CacheUse::Reuse {
            if let Some(path) = self.cached(key)? {
                return Ok(SourcesAcquired {
                    acquired: Acquired {
                        path,
                        origin: Origin::Cache,
                    },
                    attempts: Vec::new(),
                });
            }
        }
        fs::create_dir_all(&self.root).map_err(|err| {
            coded(
                ErrorCode::DownloadFailed,
                format!(
                    "cannot create cache directory {}: {err}",
                    self.root.display()
                ),
            )
        })?;
        let reference = format!("{}/{} {}", key.game_id, key.mod_id, key.version);
        let mut attempts = Vec::new();
        for source in sources {
            match &source.target {
                SourceTarget::Manual(_) => attempts.push(SourceAttempt {
                    source: source.label.clone(),
                    attempt: 1,
                    failure: Some(SourceFailure::Manual),
                }),
                SourceTarget::Refused(reason) => attempts.push(SourceAttempt {
                    source: source.label.clone(),
                    attempt: 1,
                    failure: Some(SourceFailure::Refused(reason.clone())),
                }),
                SourceTarget::Fetch(url) => {
                    let mut attempt = 0;
                    loop {
                        attempt += 1;
                        if control.checkpoint().is_err() {
                            return Err(cancelled());
                        }
                        match self.copy_source(
                            key,
                            url,
                            transport,
                            &policy,
                            package.clone(),
                            control,
                        ) {
                            Ok(()) => {
                                attempts.push(SourceAttempt {
                                    source: source.label.clone(),
                                    attempt,
                                    failure: None,
                                });
                                let url_key = CacheKey::new(
                                    key.game_id,
                                    key.mod_id,
                                    key.version,
                                    url,
                                    key.blake3,
                                );
                                return Ok(SourcesAcquired {
                                    acquired: Acquired {
                                        path: self.path(&url_key),
                                        origin: Origin::Download,
                                    },
                                    attempts,
                                });
                            }
                            Err(failure) => {
                                let retry =
                                    policy.retryable(&failure) && attempt <= policy.retries();
                                attempts.push(SourceAttempt {
                                    source: source.label.clone(),
                                    attempt,
                                    failure: Some(failure),
                                });
                                if !retry {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
        Err(coded(
            ErrorCode::DownloadFailed,
            SourcesError {
                reference,
                attempts,
            }
            .to_string(),
        ))
    }

    fn copy_source(
        &self,
        key: &CacheKey<'_>,
        url: &str,
        transport: &dyn SourceTransport,
        policy: &SourcePolicy,
        package: Option<String>,
        control: &OperationControl,
    ) -> std::result::Result<(), SourceFailure> {
        let url_key = CacheKey::new(key.game_id, key.mod_id, key.version, url, key.blake3);
        let path = self.path(&url_key);
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let mut partial = tempfile::Builder::new()
            .prefix(&format!("{name}."))
            .suffix(".download")
            .tempfile_in(&self.root)
            .map_err(|err| SourceFailure::Network(format!("cannot create partial file: {err}")))?;
        let body = transport.open(url, policy)?;
        let reader = DownloadReader::new(body.reader, body.total, package, control.clone())
            .map_err(|_| SourceFailure::Refused("operation cancelled".to_owned()))?;
        let (bytes, actual) = copy_blake3(reader, partial.as_file_mut(), MAX_DOWNLOAD_BYTES)
            .map_err(classify_read_error)?;
        if bytes > MAX_DOWNLOAD_BYTES {
            return Err(SourceFailure::Refused(format!(
                "download exceeds {MAX_DOWNLOAD_BYTES} bytes"
            )));
        }
        if actual != key.blake3 {
            return Err(SourceFailure::DigestMismatch {
                expected: key.blake3.to_owned(),
                actual,
            });
        }
        partial
            .as_file()
            .sync_all()
            .map_err(|err| SourceFailure::Network(format!("cannot flush partial file: {err}")))?;
        partial.persist(&path).map_err(|err| {
            SourceFailure::Network(format!(
                "cannot store cache entry {}: {}",
                path.display(),
                err.error
            ))
        })?;
        if let (Some(pin), Some(name)) = (ArchivePin::blake3(key.blake3), path.file_name()) {
            ArchiveIndex::record(&self.root, pin, name.to_string_lossy().into_owned());
        }
        Ok(())
    }
}

fn classify_read_error(err: anyhow::Error) -> SourceFailure {
    match err.downcast_ref::<io::Error>() {
        Some(io) if io.kind() == io::ErrorKind::TimedOut => SourceFailure::Timeout,
        _ => SourceFailure::Network(format!("{err:#}")),
    }
}

fn cancelled() -> anyhow::Error {
    coded(
        ErrorCode::Cancelled,
        "operation cancelled before the commit point",
    )
}

#[cfg(test)]
mod tests;
