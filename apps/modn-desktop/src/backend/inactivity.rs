//! Backend calls that use the inactivity timeout without `--progress json`.
//!
//! Game registration and launch do not report progress phases, so they cannot
//! use [`Backend::call_with_progress`](super::Backend::call_with_progress).
//! They still must not die at the fixed read-only deadline. Any output resets
//! the inactivity timer.
//!
//! The call ends when the CLI process exits, not when its output pipes reach
//! EOF. A started game can inherit those pipe handles on Windows and hold them
//! for hours; the CLI has already printed its result by then.
//!
//! The exit status decides first: a nonzero exit is a refusal whatever the
//! output. With exit status 0 and stdout started, a lost, oversized or
//! malformed result is the committed outcome that callers refresh.

use super::error::BackendError;
use super::progress::{self, CommittedOutcome};
use super::{Backend, OUTPUT_LIMIT};
use serde_json::Value;
use std::ffi::OsString;
use std::io::Read;
use std::process::{ChildStderr, ChildStdout, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant};

/// Default inactivity timeout when the backend has no override.
const DEFAULT_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(120);

/// Time the output readers get to deliver the result after the CLI exits.
const EOF_GRACE: Duration = Duration::from_millis(250);

const POLL_INTERVAL: Duration = Duration::from_millis(20);
const CHUNK: usize = 8 * 1024;

enum StreamItem {
    Stdout(Vec<u8>),
    StderrTruncated,
    ErrorCode(String),
    Done { stdout: bool, failed: bool },
}

/// Runs one command with the inactivity timeout and no progress protocol.
pub(super) fn call(backend: &Backend, args: &[OsString]) -> Result<Value, BackendError> {
    let timeout = backend
        .inactivity_timeout
        .unwrap_or(DEFAULT_INACTIVITY_TIMEOUT);
    let mut child = progress::spawn(backend, args, Stdio::null()).map_err(BackendError::spawn)?;
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(BackendError::io("Missing backend pipes"));
    };

    let start = Instant::now();
    let activity = Arc::new(AtomicU64::new(0));
    let (tx, rx) = mpsc::channel();
    {
        let activity = Arc::clone(&activity);
        let tx = tx.clone();
        thread::spawn(move || read_stdout(stdout, &tx, &activity, start));
    }
    {
        let activity = Arc::clone(&activity);
        thread::spawn(move || read_stderr(stderr, &tx, &activity, start));
    }

    let mut stdout_bytes = Vec::new();
    let mut stdout_started = false;
    let mut stdout_done = false;
    let mut stderr_done = false;
    let mut stdout_truncated = false;
    let mut stderr_truncated = false;
    let mut stdout_failed = false;
    let mut stderr_failed = false;
    let mut error_code = None;
    let mut timed_out = false;
    let mut exited_at = None;

    let status = loop {
        while let Ok(item) = rx.try_recv() {
            match item {
                StreamItem::Stdout(bytes) => {
                    stdout_started = true;
                    if !stdout_truncated {
                        stdout_bytes.extend_from_slice(&bytes);
                        if stdout_bytes.len() as u64 > OUTPUT_LIMIT {
                            stdout_truncated = true;
                            stdout_bytes.clear();
                        }
                    }
                }
                StreamItem::StderrTruncated => stderr_truncated = true,
                StreamItem::ErrorCode(code) => error_code = Some(code),
                StreamItem::Done { stdout, failed } => {
                    if stdout {
                        stdout_done = true;
                        stdout_failed |= failed;
                    } else {
                        stderr_done = true;
                        stderr_failed |= failed;
                    }
                }
            }
        }
        // Stdout carries the result after a write commits. Once it starts,
        // never kill the process: the files are installed and the result is
        // recoverable.
        if !stdout_started {
            let idle = start
                .elapsed()
                .saturating_sub(Duration::from_millis(activity.load(Ordering::Relaxed)));
            if idle >= timeout {
                let _ = child.kill();
                timed_out = true;
            }
            if stdout_truncated || stderr_truncated || stdout_failed || stderr_failed {
                let _ = child.kill();
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if stdout_done && stderr_done {
                    break status;
                }
                let exited = *exited_at.get_or_insert_with(Instant::now);
                if exited.elapsed() >= EOF_GRACE {
                    break status;
                }
            }
            Ok(None) => {}
            Err(_) if stdout_started => {
                // The result stream started, so the write may have committed.
                // Wait for the process instead of killing the writer.
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

    if timed_out {
        return Err(BackendError::timeout(timeout));
    }
    // The exit status comes first, as on the progress path: a failed exit is a
    // refusal even when stdout was lost. With stdout started and exit status 0,
    // the write committed, so an unreadable result is a committed outcome that
    // callers refresh rather than a plain output failure. Without stdout, an
    // output problem stays the pre-commit failure it always was.
    match progress::committed_outcome(
        status.success(),
        stdout_truncated || stdout_failed,
        !stdout_started && (stderr_truncated || stderr_failed),
    ) {
        CommittedOutcome::Refused => Err(BackendError::refused(status, error_code, None)),
        CommittedOutcome::OutputLost if stdout_started => Err(BackendError::output_after_commit()),
        CommittedOutcome::OutputLost | CommittedOutcome::StreamLost => Err(BackendError::output()),
        CommittedOutcome::Readable => {
            let stdout = String::from_utf8_lossy(&stdout_bytes);
            serde_json::from_str(&stdout).map_err(|_| {
                if stdout_started {
                    BackendError::output_after_commit()
                } else {
                    BackendError::invalid_json()
                }
            })
        }
    }
}

fn read_stdout(
    mut stdout: ChildStdout,
    tx: &Sender<StreamItem>,
    activity: &AtomicU64,
    start: Instant,
) {
    let mut chunk = [0_u8; CHUNK];
    loop {
        match stdout.read(&mut chunk) {
            Ok(0) => {
                let _ = tx.send(StreamItem::Done {
                    stdout: true,
                    failed: false,
                });
                return;
            }
            Ok(count) => {
                activity.store(start.elapsed().as_millis() as u64, Ordering::Relaxed);
                if tx
                    .send(StreamItem::Stdout(chunk[..count].to_vec()))
                    .is_err()
                {
                    return;
                }
            }
            Err(_) => {
                let _ = tx.send(StreamItem::Done {
                    stdout: true,
                    failed: true,
                });
                return;
            }
        }
    }
}

fn read_stderr(
    mut stderr: ChildStderr,
    tx: &Sender<StreamItem>,
    activity: &AtomicU64,
    start: Instant,
) {
    let mut pending = Vec::new();
    let mut chunk = [0_u8; CHUNK];
    let mut discarding = false;
    loop {
        match stderr.read(&mut chunk) {
            Ok(0) => {
                if !discarding && !pending.is_empty() {
                    emit_error_code(&pending, tx);
                }
                let _ = tx.send(StreamItem::Done {
                    stdout: false,
                    failed: false,
                });
                return;
            }
            Ok(count) => {
                activity.store(start.elapsed().as_millis() as u64, Ordering::Relaxed);
                if discarding {
                    continue;
                }
                pending.extend_from_slice(&chunk[..count]);
                while let Some(index) = pending.iter().position(|byte| *byte == b'\n') {
                    let line: Vec<u8> = pending.drain(..=index).collect();
                    emit_error_code(&line, tx);
                }
                if pending.len() as u64 > OUTPUT_LIMIT {
                    pending.clear();
                    discarding = true;
                    let _ = tx.send(StreamItem::StderrTruncated);
                }
            }
            Err(_) => {
                let _ = tx.send(StreamItem::Done {
                    stdout: false,
                    failed: true,
                });
                return;
            }
        }
    }
}

fn emit_error_code(line: &[u8], tx: &Sender<StreamItem>) {
    let text = String::from_utf8_lossy(line);
    let Ok(value) = serde_json::from_str::<Value>(text.trim()) else {
        return;
    };
    if let Some(code) = value.get("error").and_then(Value::as_str) {
        let _ = tx.send(StreamItem::ErrorCode(code.to_owned()));
    }
}
