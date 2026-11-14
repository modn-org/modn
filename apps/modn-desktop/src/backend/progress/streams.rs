//! The stderr progress and stdout result readers of a progress-aware call.

use super::super::OUTPUT_LIMIT;
use super::super::error::ErrorEnvelope;
use super::ProgressEvent;
use std::io::{BufReader, Read};
use std::process::{ChildStderr, ChildStdout};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::time::Instant;

/// One item the stderr reader sends to the caller.
pub(super) enum StderrItem {
    /// One progress event the CLI wrote.
    Event(ProgressEvent),
    /// One CLI error envelope with the refusal's uncovered capabilities.
    Envelope(ErrorEnvelope),
    /// The stream passed the output bound; the reader keeps draining it.
    Oversized,
    /// The reader reached the end of the stream.
    Done {
        /// Whether reading failed before the end.
        failed: bool,
    },
}

/// One item the stdout reader sends to the caller.
pub(super) enum StdoutItem {
    /// The stream passed the output bound; the reader keeps draining it.
    Oversized,
    /// The reader reached the end of the stream.
    Done {
        /// The bytes read, empty when the stream was oversized or failed.
        bytes: Vec<u8>,
        /// Whether reading failed before the end.
        failed: bool,
    },
}

const CHUNK: usize = 8 * 1024;

pub(super) fn read_stderr(
    stderr: ChildStderr,
    tx: Sender<StderrItem>,
    activity: Arc<AtomicU64>,
    start: Instant,
) {
    let mut reader = BufReader::new(stderr);
    let mut pending = Vec::new();
    let mut chunk = [0_u8; CHUNK];
    let mut discarding = false;
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => {
                if !discarding && !pending.is_empty() {
                    emit_line(&pending, &tx);
                }
                let _ = tx.send(StderrItem::Done { failed: false });
                return;
            }
            Ok(count) => {
                touch(&activity, start);
                if discarding {
                    continue;
                }
                pending.extend_from_slice(&chunk[..count]);
                while let Some(index) = pending.iter().position(|byte| *byte == b'\n') {
                    let line: Vec<u8> = pending.drain(..=index).collect();
                    emit_line(&line, &tx);
                }
                if pending.len() as u64 > OUTPUT_LIMIT {
                    pending.clear();
                    discarding = true;
                    let _ = tx.send(StderrItem::Oversized);
                }
            }
            Err(_) => {
                let _ = tx.send(StderrItem::Done { failed: true });
                return;
            }
        }
    }
}

pub(super) fn read_stdout(
    stdout: ChildStdout,
    tx: Sender<StdoutItem>,
    activity: Arc<AtomicU64>,
    start: Instant,
) {
    let mut reader = stdout;
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; CHUNK];
    let mut oversized = false;
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => {
                touch(&activity, start);
                if oversized {
                    continue;
                }
                bytes.extend_from_slice(&chunk[..count]);
                if bytes.len() as u64 > OUTPUT_LIMIT {
                    // Keep the pipe moving so a writer that committed is not
                    // blocked, and free the unusable result as it arrives.
                    oversized = true;
                    bytes = Vec::new();
                    let _ = tx.send(StdoutItem::Oversized);
                }
            }
            Err(_) => {
                let _ = tx.send(StdoutItem::Done {
                    bytes: Vec::new(),
                    failed: true,
                });
                return;
            }
        }
    }
    let _ = tx.send(StdoutItem::Done {
        bytes,
        failed: false,
    });
}

fn touch(activity: &AtomicU64, start: Instant) {
    activity.store(start.elapsed().as_millis() as u64, Ordering::Relaxed);
}

fn emit_line(line: &[u8], tx: &Sender<StderrItem>) {
    let text = String::from_utf8_lossy(line);
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };
    if let Some(event) = ProgressEvent::parse(&value) {
        let _ = tx.send(StderrItem::Event(event));
    } else if let Some(envelope) = ErrorEnvelope::parse(&value) {
        let _ = tx.send(StderrItem::Envelope(envelope));
    }
}
