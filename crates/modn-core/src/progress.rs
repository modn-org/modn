//! Progress events and cooperative cancellation for managed operations.
//!
//! A front end creates one [`OperationControl`] per operation, such as an
//! install, update, uninstall or download. The operation reports each phase
//! through it and checks its [`CancellationToken`] at safe points.
//!
//! # Contract
//!
//! 1. The caller calls [`OperationControl::begin`] at the start of the
//!    resolve phase, before it reads catalogs or plans files.
//! 2. The download step reports [`ProgressEvent::Download`] events with the
//!    bytes received and the expected total, when the source states one.
//! 3. The transaction reports [`ProgressEvent::Extract`], then
//!    [`ProgressEvent::Stage`], then [`ProgressEvent::Commit`].
//! 4. The caller calls [`OperationControl::finish`] with the outcome, or
//!    [`OperationControl::finish_error`] when the operation failed.
//!
//! [`ProgressEvent::Commit`] is the commit point: before it, a cancellation
//! stops the operation with [`Cancelled`], rolls back staging or a withdrawn
//! update, and leaves the game files, manager state and ownership records as
//! they were. From it on, Modn ignores cancellation and finishes or rolls back
//! the operation as it does without one.
//!
//! A library caller may pass a control to an install, update, uninstall or
//! [`crate::acquisition::ArchiveCache::acquire_with_control`] without a CLI.

use crate::error::ErrorCode;
use serde::Serialize;
use std::fmt;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A shared flag that asks an operation to stop before its commit point.
///
/// Clones share one flag, so another thread can cancel the operation.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    /// Creates a token that is not cancelled.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks the operation to stop. Has no effect after the commit point.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Whether [`CancellationToken::cancel`] was called.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// How an operation ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Outcome {
    /// The operation finished.
    Succeeded,
    /// The operation failed; the error code says why.
    Failed,
    /// The operation stopped before its commit point.
    Cancelled,
}

/// One phase of a managed operation.
///
/// It serializes as one JSON object with a `phase` field, for example
/// `{"phase":"download","package":"main/game/mod","bytes":0,"total":4096}`.
/// `package` is the catalog reference, or the managed-package ID when the
/// package has none, and is absent when the phase has no single package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProgressEvent {
    /// The operation resolves its target and plan.
    Resolve,
    /// An archive download advanced to `bytes` of `total`.
    Download {
        /// The package whose archive downloads.
        #[serde(skip_serializing_if = "Option::is_none")]
        package: Option<String>,
        /// Bytes received so far.
        bytes: u64,
        /// The expected size, or `null` when the source does not say.
        total: Option<u64>,
    },
    /// Modn extracts an archive into manager-owned staging.
    Extract {
        /// The package whose archive extracts.
        #[serde(skip_serializing_if = "Option::is_none")]
        package: Option<String>,
    },
    /// Modn verifies the staged content and records the staged transaction.
    Stage {
        /// The package whose content stages.
        #[serde(skip_serializing_if = "Option::is_none")]
        package: Option<String>,
    },
    /// The commit point: Modn starts to change the game.
    Commit {
        /// The package that the commit changes.
        #[serde(skip_serializing_if = "Option::is_none")]
        package: Option<String>,
    },
    /// The operation ended.
    Result {
        /// How it ended.
        outcome: Outcome,
        /// The error code when it did not succeed.
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<ErrorCode>,
    },
}

/// The error of an operation cancelled before its commit point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("operation cancelled before the commit point; nothing changed")
    }
}

impl std::error::Error for Cancelled {}

type Reporter = dyn Fn(&ProgressEvent) + Send + Sync;

struct Inner {
    token: CancellationToken,
    reporter: Option<Box<Reporter>>,
    committed: AtomicBool,
}

/// Reports progress and observes cancellation for one operation.
///
/// Clones share the same token, reporter and commit state, so one operation
/// can hand the control to each step. [`OperationControl::default`] reports
/// nothing and is never cancelled.
#[derive(Clone)]
pub struct OperationControl(Arc<Inner>);

impl Default for OperationControl {
    fn default() -> Self {
        Self(Arc::new(Inner {
            token: CancellationToken::new(),
            reporter: None,
            committed: AtomicBool::new(false),
        }))
    }
}

impl fmt::Debug for OperationControl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OperationControl")
            .field("cancelled", &self.0.token.is_cancelled())
            .field("committed", &self.is_committed())
            .finish_non_exhaustive()
    }
}

impl OperationControl {
    /// Creates a control that passes each event to `reporter`.
    #[must_use]
    pub fn new(
        token: CancellationToken,
        reporter: impl Fn(&ProgressEvent) + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(Inner {
            token,
            reporter: Some(Box::new(reporter)),
            committed: AtomicBool::new(false),
        }))
    }

    /// The token that cancels this operation.
    #[must_use]
    pub fn token(&self) -> &CancellationToken {
        &self.0.token
    }

    /// Whether the operation passed its commit point.
    #[must_use]
    pub fn is_committed(&self) -> bool {
        self.0.committed.load(Ordering::SeqCst)
    }

    /// Stops the operation when it is cancelled before the commit point.
    ///
    /// # Errors
    ///
    /// Returns [`Cancelled`] when the token is cancelled and the operation
    /// has not passed its commit point.
    pub fn checkpoint(&self) -> Result<(), Cancelled> {
        if self.0.token.is_cancelled() && !self.is_committed() {
            return Err(Cancelled);
        }
        Ok(())
    }

    /// Reports the resolve phase after a cancellation check.
    ///
    /// # Errors
    ///
    /// Returns [`Cancelled`] when the operation is already cancelled.
    pub fn begin(&self) -> Result<(), Cancelled> {
        self.advance(ProgressEvent::Resolve)
    }

    /// Reports the result of the operation.
    ///
    /// A result event is always reported, also after a cancellation or the
    /// commit point.
    pub fn finish(&self, outcome: Outcome, error: Option<ErrorCode>) {
        self.report(&ProgressEvent::Result { outcome, error });
    }

    /// Reports a failed result with its stable `code`.
    pub fn finish_error(&self, code: ErrorCode) {
        self.finish(Outcome::Failed, Some(code));
    }

    /// Reports a result cancelled before the commit point.
    pub fn finish_cancelled(&self) {
        self.finish(Outcome::Cancelled, Some(ErrorCode::Cancelled));
    }

    /// Reports `event` after a cancellation check.
    ///
    /// A [`ProgressEvent::Commit`] event passes the commit point: the check
    /// happens once, and later checks always succeed. A
    /// [`ProgressEvent::Result`] event is reported without a check.
    ///
    /// # Errors
    ///
    /// Returns [`Cancelled`], without reporting `event`, when
    /// [`OperationControl::checkpoint`] fails.
    pub fn advance(&self, event: ProgressEvent) -> Result<(), Cancelled> {
        match event {
            ProgressEvent::Result { .. } => {}
            ProgressEvent::Commit { .. } => {
                self.checkpoint()?;
                self.0.committed.store(true, Ordering::SeqCst);
            }
            _ => self.checkpoint()?,
        }
        self.report(&event);
        Ok(())
    }

    /// Passes `event` to the reporter without a cancellation check.
    pub fn report(&self, event: &ProgressEvent) {
        if let Some(reporter) = &self.0.reporter {
            reporter(event);
        }
    }
}

/// The download progress step: one event per MiB received, at most.
const DOWNLOAD_REPORT_BYTES: u64 = 1024 * 1024;

/// A response body that reports [`ProgressEvent::Download`] events and stops
/// when its operation is cancelled.
///
/// Pass it as the body that [`crate::acquisition::ArchiveCache::acquire`]
/// fetches. A cancelled read fails with an I/O error that wraps
/// [`Cancelled`], so the cache removes the partial file and keeps no entry.
/// It reports at the start, after each MiB, and at the end of the body.
#[derive(Debug)]
pub struct DownloadReader<R> {
    inner: R,
    control: OperationControl,
    package: Option<String>,
    bytes: u64,
    total: Option<u64>,
    reported: u64,
}

impl<R: Read> DownloadReader<R> {
    /// Wraps `inner`, a body of `total` bytes when the source says so, and
    /// reports the download of `package` at zero bytes.
    ///
    /// # Errors
    ///
    /// Returns [`Cancelled`] when the operation is already cancelled.
    pub fn new(
        inner: R,
        total: Option<u64>,
        package: Option<String>,
        control: OperationControl,
    ) -> Result<Self, Cancelled> {
        control.advance(ProgressEvent::Download {
            package: package.clone(),
            bytes: 0,
            total,
        })?;
        Ok(Self {
            inner,
            control,
            package,
            bytes: 0,
            total,
            reported: 0,
        })
    }
}

impl<R: Read> Read for DownloadReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.control.checkpoint().map_err(io::Error::other)?;
        let read = self.inner.read(buffer)?;
        self.bytes += read as u64;
        let unreported = self.bytes - self.reported;
        if unreported >= DOWNLOAD_REPORT_BYTES || (read == 0 && unreported > 0) {
            self.reported = self.bytes;
            self.control
                .advance(ProgressEvent::Download {
                    package: self.package.clone(),
                    bytes: self.bytes,
                    total: self.total,
                })
                .map_err(io::Error::other)?;
        }
        Ok(read)
    }
}

#[cfg(test)]
mod tests;
