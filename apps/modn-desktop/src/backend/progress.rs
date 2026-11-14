//! Progress, cancellation and the single process-spawn helper of the desktop backend.
//!
//! The CLI writes one JSON progress event per line on stderr while it runs a
//! managed write. This module reads those lines, forwards the events to the
//! caller in order, and writes `cancel` on stdin when the caller asks it to.
//! Stdout keeps its normal single JSON result.

use super::Backend;
use super::error::BackendError;
use serde::Serialize;
use serde_json::Value;
use std::ffi::OsString;
use std::io::Write;
use std::process::{ChildStdin, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

#[path = "progress/spawn.rs"]
mod spawn;
#[path = "progress/streams.rs"]
mod streams;

pub(super) use spawn::spawn;

use streams::{StderrItem, StdoutItem, read_stderr, read_stdout};

/// Default inactivity timeout for [`Backend::call_with_progress`].
const DEFAULT_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(120);

/// Time the CLI gets to stop after a timeout sent `cancel` before it is killed.
const CANCEL_GRACE: Duration = Duration::from_secs(15);

const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// A shared flag that asks a running operation to stop before its commit point.
///
/// Clones share one flag, so another thread can cancel the operation while
/// [`Backend::call_with_progress`] waits for the CLI. Cancelling never kills the
/// process: the desktop writes one `cancel` line on stdin and the CLI stops at
/// its next safe point.
#[derive(Clone, Debug, Default)]
pub(crate) struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// Creates a token that is not cancelled.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Asks the operation to stop. Has no effect after the CLI's commit point.
    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Whether [`CancelToken::cancel`] was called.
    #[must_use]
    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// One progress event that the CLI wrote on stderr.
///
/// The CLI serializes each event as one JSON object with a `phase` field.
/// Fields that the phase does not use stay `None`, so unknown future phases
/// still reach the caller.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ProgressEvent {
    /// Phase name, for example `resolve`, `download`, `commit` or `result`.
    pub phase: String,
    /// Package reference or managed-package ID, when the phase names one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    /// Bytes received so far, for a `download` event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    /// Expected size, when the source states one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    /// Terminal outcome, for a `result` event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// Stable error code, for a failed `result` event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ProgressEvent {
    fn parse(value: &Value) -> Option<Self> {
        let phase = value.get("phase")?.as_str()?.to_owned();
        Some(Self {
            phase,
            package: value
                .get("package")
                .and_then(Value::as_str)
                .map(str::to_owned),
            bytes: value.get("bytes").and_then(Value::as_u64),
            total: value.get("total").and_then(Value::as_u64),
            outcome: value
                .get("outcome")
                .and_then(Value::as_str)
                .map(str::to_owned),
            error: value
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
    }
}

/// The outcome to report once the CLI reported its commit event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommittedOutcome {
    /// The CLI exited with a failure, so the commit did not complete.
    Refused,
    /// The committed result could not be read.
    OutputLost,
    /// stderr could not be read, so the operation has no progress warning.
    StreamLost,
    /// stdout was read within its bound and still has to be parsed.
    Readable,
}

/// Decides how a call that saw the commit event ends.
///
/// The exit status comes first: a `commit` event alone does not prove that the
/// write completed, so a failed exit is a refusal even when the result stream
/// was lost. A lost stream with a successful exit is a committed outcome that
/// callers refresh, never plain success. So is a readable stream whose bytes
/// are not one JSON value: the write is installed and only its result is
/// unknown.
fn committed_outcome(exit_ok: bool, stdout_lost: bool, stderr_lost: bool) -> CommittedOutcome {
    if !exit_ok {
        CommittedOutcome::Refused
    } else if stdout_lost {
        CommittedOutcome::OutputLost
    } else if stderr_lost {
        CommittedOutcome::StreamLost
    } else {
        CommittedOutcome::Readable
    }
}

/// Whether the backend process may be killed on this path.
///
/// A kill is only allowed before the commit event. After it, the files are
/// installed and the CLI may still be finishing its own result or rollback, so
/// every path waits or polls instead.
fn may_kill(committed: bool) -> bool {
    !committed
}

/// Runs one write command with `--progress json` and cooperative cancellation.
pub(super) fn call(
    backend: &Backend,
    args: &[OsString],
    cancel: &CancelToken,
    on_event: &mut dyn FnMut(ProgressEvent),
) -> Result<Value, BackendError> {
    let timeout = backend
        .inactivity_timeout
        .unwrap_or(DEFAULT_INACTIVITY_TIMEOUT);
    let mut args = args.to_vec();
    args.extend([OsString::from("--progress"), OsString::from("json")]);
    let mut child = spawn(backend, &args, Stdio::piped()).map_err(BackendError::spawn)?;
    let (Some(stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(BackendError::io("Missing backend pipes"));
    };

    let start = Instant::now();
    let activity = Arc::new(AtomicU64::new(0));
    let (stderr_tx, stderr_rx) = mpsc::channel();
    let (stdout_tx, stdout_rx) = mpsc::channel();
    {
        let activity = Arc::clone(&activity);
        thread::spawn(move || read_stderr(stderr, stderr_tx, activity, start));
    }
    {
        let activity = Arc::clone(&activity);
        thread::spawn(move || read_stdout(stdout, stdout_tx, activity, start));
    }

    let mut stdin = stdin;
    let mut cancel_sent = false;
    let mut user_cancelled = false;
    let mut committed = false;
    let mut timed_out = false;
    let mut killed = false;
    let mut timeout_deadline = None;
    let mut stderr_done = false;
    let mut stdout_done = false;
    let mut stdout_bytes = None;
    let mut stream_failed = false;
    let mut stderr_failed = false;
    let mut truncated = false;
    let mut stdout_truncated = false;
    let mut stdout_failed = false;
    let mut error_code = None;
    let mut approval = None;

    let status = loop {
        while let Ok(item) = stderr_rx.try_recv() {
            match item {
                StderrItem::Event(event) => {
                    if event.phase == "commit" {
                        committed = true;
                        timeout_deadline = None;
                    }
                    if event.phase == "result"
                        && let Some(code) = &event.error
                    {
                        error_code = Some(code.clone());
                    }
                    on_event(event);
                }
                StderrItem::Envelope(envelope) => {
                    error_code = Some(envelope.code);
                    approval = envelope.approval;
                }
                StderrItem::Oversized => truncated = true,
                StderrItem::Done { failed } => {
                    stderr_done = true;
                    stderr_failed |= failed;
                    stream_failed |= failed;
                }
            }
        }
        while let Ok(item) = stdout_rx.try_recv() {
            match item {
                StdoutItem::Oversized => {
                    truncated = true;
                    stdout_truncated = true;
                }
                StdoutItem::Done { bytes, failed } => {
                    stdout_done = true;
                    if failed {
                        stream_failed = true;
                        stdout_failed = true;
                    } else if !stdout_truncated {
                        stdout_bytes = Some(bytes);
                    }
                }
            }
        }
        // A user cancel wins even when the inactivity path sent `cancel` first.
        if cancel.is_cancelled() {
            user_cancelled = true;
            if !cancel_sent {
                send_cancel(&mut stdin);
                cancel_sent = true;
            }
        }
        // A cancel stops the CLI from starting new work, but a step already
        // past its last progress event can still stall. Keep the pre-commit
        // inactivity deadline armed so a silent step cannot block forever.
        if !committed && timeout_deadline.is_none() {
            let idle = start
                .elapsed()
                .saturating_sub(Duration::from_millis(activity.load(Ordering::Relaxed)));
            if idle >= timeout {
                if !cancel_sent {
                    send_cancel(&mut stdin);
                    cancel_sent = true;
                    timed_out = true;
                }
                timeout_deadline = Some(Instant::now() + CANCEL_GRACE);
            }
        }
        if timeout_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            if may_kill(committed) {
                let _ = child.kill();
                killed = true;
            }
            timeout_deadline = None;
        }
        // The readers keep draining after an oversized stream, so this kill is
        // what stops an unbounded or unreadable writer before the commit.
        // Never kill after the commit event: the files are installed and the
        // result is recoverable.
        if may_kill(committed) && (truncated || stream_failed) {
            let _ = child.kill();
        }
        match child.try_wait() {
            Ok(Some(status)) if stderr_done && stdout_done => break status,
            Ok(Some(_) | None) => {}
            Err(_) if !may_kill(committed) => {
                // The commit is durable. Wait for the process instead of
                // killing it; a failed wait leaves the outcome uncertain, so
                // report a committed refresh outcome rather than success.
                match child.wait() {
                    Ok(status) => break status,
                    Err(_) => return Err(BackendError::output_after_commit()),
                }
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(BackendError::io("Backend process state could not be read"));
            }
        }
        thread::sleep(POLL_INTERVAL);
    };
    drop(stdin);

    if committed {
        return match committed_outcome(
            status.success(),
            stdout_truncated || stdout_failed,
            stderr_failed,
        ) {
            CommittedOutcome::Refused => Err(BackendError::refused(status, error_code, approval)),
            CommittedOutcome::OutputLost | CommittedOutcome::StreamLost => {
                Err(BackendError::output_after_commit())
            }
            CommittedOutcome::Readable => {
                let bytes = stdout_bytes.ok_or_else(BackendError::output_after_commit)?;
                let stdout = String::from_utf8_lossy(&bytes);
                serde_json::from_str(&stdout).map_err(|_| BackendError::output_after_commit())
            }
        };
    }
    if killed || timed_out {
        return Err(if user_cancelled {
            BackendError::cancelled()
        } else {
            BackendError::timeout(timeout)
        });
    }
    if stream_failed || truncated {
        return Err(BackendError::output());
    }
    if !status.success() {
        if user_cancelled || cancel.is_cancelled() {
            return Err(BackendError::cancelled());
        }
        return Err(BackendError::refused(status, error_code, approval));
    }
    let bytes = stdout_bytes.ok_or_else(BackendError::output)?;
    let stdout = String::from_utf8_lossy(&bytes);
    serde_json::from_str(&stdout).map_err(|_| BackendError::invalid_json())
}

fn send_cancel(stdin: &mut ChildStdin) {
    let _ = stdin.write_all(b"cancel\n");
    let _ = stdin.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_exit_after_commit_wins_over_lost_output() {
        assert_eq!(
            committed_outcome(false, true, true),
            CommittedOutcome::Refused,
            "a commit event alone does not prove the write completed"
        );
    }

    #[test]
    fn a_stderr_failure_after_commit_is_not_plain_success() {
        assert_eq!(
            committed_outcome(true, false, true),
            CommittedOutcome::StreamLost,
            "a committed result with an unreadable progress stream needs a refresh outcome"
        );
        assert_eq!(
            committed_outcome(true, false, false),
            CommittedOutcome::Readable
        );
    }

    #[test]
    fn a_committed_backend_is_never_killed() {
        assert!(!may_kill(true), "no path may kill after the commit event");
        assert!(may_kill(false), "a pre-commit stall may be killed");
    }
}
