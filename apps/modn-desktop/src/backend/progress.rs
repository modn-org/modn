//! Progress, cancellation and the single process-spawn helper of the desktop backend.
//!
//! The CLI writes one JSON progress event per line on stderr while it runs a
//! managed write. This module reads those lines, forwards the events to the
//! caller in order, and writes `cancel` on stdin when the caller asks it to.
//! Stdout keeps its normal single JSON result.

#![allow(
    dead_code,
    reason = "The desktop UI wires this API in a parallel change; the contract tests exercise it"
)]

use super::error::BackendError;
use super::{Backend, OUTPUT_LIMIT};
use serde::Serialize;
use serde_json::Value;
use std::ffi::{OsStr, OsString};
use std::io::{self, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant};

/// Default inactivity timeout for [`Backend::call_with_progress`].
const DEFAULT_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(120);

/// Time the CLI gets to stop after a timeout sent `cancel` before it is killed.
const CANCEL_GRACE: Duration = Duration::from_secs(15);

const POLL_INTERVAL: Duration = Duration::from_millis(20);
const CHUNK: usize = 8 * 1024;

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

enum StderrItem {
    Event(ProgressEvent),
    ErrorCode(String),
    Done { failed: bool },
}

struct StdoutItem {
    bytes: Vec<u8>,
    failed: bool,
}

/// Starts the configured CLI with the fixed argument prefix.
///
/// This is the only place in the crate that creates a [`Command`]. The child
/// environment is cleared, then only these variables are forwarded from the
/// app's own environment when they are set:
///
/// - `SystemRoot`: process basics on Windows.
/// - `TEMP` and `TMP`: only when the value is an absolute Windows path. A
///   POSIX value such as Git Bash's `/tmp`, or a drive-relative value, is left
///   out so the CLI keeps its default instead of resolving the path against
///   the current drive.
/// - `LOCALAPPDATA`: Modn's data directory, unless `--data-dir` overrides it.
/// - `MODN_CACHE`, `MODN_STORE_LIBRARIES` and `MODN_APPDATA`: disposable
///   fixture overrides.
/// - `MODN_GAME_MODULES_DIR`: the module source, unless `--module-dir`
///   overrides it.
pub(super) fn spawn(backend: &Backend, args: &[OsString], stdin: Stdio) -> io::Result<Child> {
    let mut command = Command::new(&backend.binary);
    command.args([
        OsString::from("--json"),
        OsString::from("--state-dir"),
        backend.state.clone().into_os_string(),
    ]);
    command
        .args(args)
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.env_clear();
    for name in [
        "SystemRoot",
        "MODN_CACHE",
        "MODN_STORE_LIBRARIES",
        "MODN_APPDATA",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    for name in ["TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(name)
            && is_absolute_windows_path(&value)
        {
            command.env(name, value);
        }
    }
    if let Some(modules) = &backend.modules {
        command.env("MODN_GAME_MODULES_DIR", modules);
    } else if let Some(value) = std::env::var_os("MODN_GAME_MODULES_DIR") {
        command.env("MODN_GAME_MODULES_DIR", value);
    }
    if let Some(data) = &backend.data {
        command.env("LOCALAPPDATA", data);
    } else if let Some(value) = std::env::var_os("LOCALAPPDATA") {
        command.env("LOCALAPPDATA", value);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.spawn()
}

/// Whether `value` names an absolute Windows path.
///
/// [`Path::is_absolute`] needs a drive prefix or a UNC share on Windows, so
/// `/tmp` and `\tmp` fail while `C:\tmp` and `\\server\share` pass. Only such
/// values are safe to hand to a Windows child process.
fn is_absolute_windows_path(value: &OsStr) -> bool {
    Path::new(value).is_absolute()
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
    let mut committed = false;
    let mut timed_out = false;
    let mut killed = false;
    let mut timeout_deadline = None;
    let mut stderr_done = false;
    let mut stdout_done = false;
    let mut stdout_bytes = None;
    let mut stream_failed = false;
    let mut error_code = None;

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
                StderrItem::ErrorCode(code) => error_code = Some(code),
                StderrItem::Done { failed } => {
                    stderr_done = true;
                    stream_failed |= failed;
                }
            }
        }
        while let Ok(item) = stdout_rx.try_recv() {
            stdout_done = true;
            if item.failed || item.bytes.len() as u64 > OUTPUT_LIMIT {
                stream_failed = true;
            } else {
                stdout_bytes = Some(item.bytes);
            }
        }
        if !cancel_sent && cancel.is_cancelled() {
            send_cancel(&mut stdin);
            cancel_sent = true;
        }
        if !committed && !cancel_sent && timeout_deadline.is_none() {
            let idle = start
                .elapsed()
                .saturating_sub(Duration::from_millis(activity.load(Ordering::Relaxed)));
            if idle >= timeout {
                send_cancel(&mut stdin);
                cancel_sent = true;
                timed_out = true;
                timeout_deadline = Some(Instant::now() + CANCEL_GRACE);
            }
        }
        if timeout_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            let _ = child.kill();
            killed = true;
            timeout_deadline = None;
        }
        if stream_failed {
            let _ = child.kill();
        }
        match child.try_wait() {
            Ok(Some(status)) if stderr_done && stdout_done => break status,
            Ok(Some(_) | None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(BackendError::io("Backend process state could not be read"));
            }
        }
        thread::sleep(POLL_INTERVAL);
    };
    drop(stdin);

    if killed || (timed_out && !committed) {
        return Err(BackendError::timeout(timeout));
    }
    if stream_failed {
        return Err(BackendError::output());
    }
    if !status.success() {
        if cancel.is_cancelled() && !committed {
            return Err(BackendError::cancelled());
        }
        return Err(BackendError::refused(status, error_code));
    }
    let bytes = stdout_bytes.ok_or_else(BackendError::output)?;
    let stdout = String::from_utf8_lossy(&bytes);
    serde_json::from_str(&stdout).map_err(|_| BackendError::invalid_json())
}

fn send_cancel(stdin: &mut ChildStdin) {
    let _ = stdin.write_all(b"cancel\n");
    let _ = stdin.flush();
}

fn touch(activity: &AtomicU64, start: Instant) {
    activity.store(start.elapsed().as_millis() as u64, Ordering::Relaxed);
}

fn read_stderr(
    stderr: ChildStderr,
    tx: Sender<StderrItem>,
    activity: Arc<AtomicU64>,
    start: Instant,
) {
    let mut reader = BufReader::new(stderr);
    let mut pending = Vec::new();
    let mut chunk = [0_u8; CHUNK];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => {
                if !pending.is_empty() {
                    emit_line(&pending, &tx);
                }
                let _ = tx.send(StderrItem::Done { failed: false });
                return;
            }
            Ok(count) => {
                touch(&activity, start);
                pending.extend_from_slice(&chunk[..count]);
                while let Some(index) = pending.iter().position(|byte| *byte == b'\n') {
                    let line: Vec<u8> = pending.drain(..=index).collect();
                    emit_line(&line, &tx);
                }
                if pending.len() as u64 > OUTPUT_LIMIT {
                    let _ = tx.send(StderrItem::Done { failed: true });
                    return;
                }
            }
            Err(_) => {
                let _ = tx.send(StderrItem::Done { failed: true });
                return;
            }
        }
    }
}

fn emit_line(line: &[u8], tx: &Sender<StderrItem>) {
    let text = String::from_utf8_lossy(line);
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return;
    };
    if let Some(event) = ProgressEvent::parse(&value) {
        let _ = tx.send(StderrItem::Event(event));
    } else if let Some(code) = value.get("error").and_then(Value::as_str) {
        let _ = tx.send(StderrItem::ErrorCode(code.to_owned()));
    }
}

fn read_stdout(
    stdout: ChildStdout,
    tx: Sender<StdoutItem>,
    activity: Arc<AtomicU64>,
    start: Instant,
) {
    let mut reader = stdout.take(OUTPUT_LIMIT + 1);
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; CHUNK];
    let mut failed = false;
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => {
                touch(&activity, start);
                bytes.extend_from_slice(&chunk[..count]);
            }
            Err(_) => {
                failed = true;
                break;
            }
        }
    }
    let _ = tx.send(StdoutItem { bytes, failed });
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn only_drive_qualified_temp_paths_are_forwarded() {
        for absolute in [r"C:\Users\J\AppData\Local\Temp", r"\\server\share\tmp"] {
            assert!(
                is_absolute_windows_path(OsStr::new(absolute)),
                "expected an absolute Windows path: {absolute}"
            );
        }
        for relative in ["/tmp", r"\tmp", "tmp", r"C:tmp"] {
            assert!(
                !is_absolute_windows_path(OsStr::new(relative)),
                "must not forward a non-absolute Windows path: {relative}"
            );
        }
    }
}
