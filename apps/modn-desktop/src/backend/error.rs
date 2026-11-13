//! The error type of [`Backend::call_with_progress`](super::Backend::call_with_progress).

#![allow(
    dead_code,
    reason = "The desktop UI wires this API in a parallel change; the contract tests exercise it"
)]

use std::io;
use std::process::ExitStatus;
use std::time::Duration;

/// A failed progress-aware backend call.
///
/// The message never contains raw CLI stderr text. Callers render
/// [`BackendError::code`] through their own short-message map.
#[derive(Debug)]
pub(crate) struct BackendError {
    kind: BackendErrorKind,
    code: Option<String>,
    message: String,
}

impl BackendError {
    pub(super) fn cancelled() -> Self {
        Self {
            kind: BackendErrorKind::Cancelled,
            code: Some("CANCELLED".into()),
            message: "Backend cancelled before the commit point; nothing changed".into(),
        }
    }

    pub(super) fn timeout(timeout: Duration) -> Self {
        Self {
            kind: BackendErrorKind::Timeout,
            code: None,
            message: format!(
                "Backend made no progress for {:.1} seconds before its commit point; the operation was cancelled and stopped",
                timeout.as_secs_f64()
            ),
        }
    }

    pub(super) fn refused(exit: ExitStatus, code: Option<String>) -> Self {
        let message = match &code {
            Some(code) => format!("Backend refused ({exit}): {code}"),
            None => format!("Backend refused ({exit})"),
        };
        Self {
            kind: BackendErrorKind::Refused,
            code,
            message,
        }
    }

    pub(super) fn output() -> Self {
        Self {
            kind: BackendErrorKind::Output,
            code: None,
            message: "Backend output exceeded its bound or could not be read".into(),
        }
    }

    pub(super) fn spawn(error: io::Error) -> Self {
        Self {
            kind: BackendErrorKind::Spawn,
            code: None,
            message: format!("Cannot start configured backend: {error}"),
        }
    }

    pub(super) fn io(message: &str) -> Self {
        Self {
            kind: BackendErrorKind::Output,
            code: None,
            message: message.into(),
        }
    }

    pub(super) fn invalid_json() -> Self {
        Self {
            kind: BackendErrorKind::InvalidJson,
            code: None,
            message: "Backend returned invalid JSON; verify the configured CLI version".into(),
        }
    }

    /// Why the call failed.
    #[must_use]
    pub(crate) fn kind(&self) -> BackendErrorKind {
        self.kind
    }

    /// The CLI's stable error code, when it reported one.
    ///
    /// A run cancelled before the commit point reports `CANCELLED` even when
    /// the CLI exits without its error envelope.
    #[must_use]
    pub(crate) fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for BackendError {}

/// Why a progress-aware backend call failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BackendErrorKind {
    /// The CLI stopped before the commit point because the caller cancelled.
    Cancelled,
    /// The CLI produced no output before the inactivity timeout, so it was stopped.
    Timeout,
    /// The CLI exited with a failure.
    Refused,
    /// A stream exceeded its bound or could not be read.
    Output,
    /// The configured executable could not start.
    Spawn,
    /// The CLI exited successfully but stdout was not one JSON value.
    InvalidJson,
}
