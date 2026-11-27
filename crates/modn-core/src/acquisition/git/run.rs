//! Run one Git child with fixed memory for its output.
//!
//! Git writes progress and remote messages to its error stream. A hostile
//! remote can keep that stream busy without limit, so Modn drains both piped
//! streams continuously and keeps only the last [`OUTPUT_TAIL_LIMIT`] bytes of
//! each. The low-speed marker is detected while draining, because a flood of
//! later messages can push it out of the kept tail.
//!
//! The wait loop also observes the operation's cancellation token, so a
//! cancelled operation stops a slow fetch or clone at once instead of waiting
//! for Git to return.

use crate::progress::CancellationToken;
use std::io::{self, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::Duration;

/// The most bytes Modn keeps from the end of each piped Git stream.
pub(crate) const OUTPUT_TAIL_LIMIT: usize = 64 * 1024;

/// Marker text that classifies a Git low-speed abort, matched in lowercase.
const LOW_SPEED_MARKERS: [&[u8]; 2] = [b"operation too slow", b"bytes/sec transferred"];

/// The longest marker, less one byte, carries a marker split across reads.
const MARKER_CARRY: usize = 20;

const READ_CHUNK: usize = 8 * 1024;

/// How long the wait loop sleeps before it checks the child and the token.
pub(crate) const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Why the runner returned no finished child.
#[derive(Debug)]
pub(crate) enum RunError {
    /// The operation token was cancelled before the child finished.
    Cancelled,
    /// The operating system could not spawn, poll or reap the child.
    Io(io::Error),
}

/// One finished Git child and the bounded version of what it printed.
pub(crate) struct GitOutput {
    pub(crate) status: ExitStatus,
    /// Kept standard output tail. Git progress goes to its error stream, so
    /// no error message reads this field yet.
    #[expect(
        dead_code,
        reason = "the runner keeps both piped streams bounded, including stdout"
    )]
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) truncated: bool,
    pub(crate) low_speed: bool,
}

impl GitOutput {
    pub(crate) fn success(&self) -> bool {
        self.status.success()
    }

    /// The kept error text for an error message.
    ///
    /// A truncated stream says so before the kept tail, so the message stays
    /// near the limit even when Git printed far more.
    pub(crate) fn error_text(&self) -> String {
        if !self.truncated {
            return String::from_utf8_lossy(&self.stderr).trim().to_owned();
        }
        let mut text = format!(
            "Git output exceeded the {OUTPUT_TAIL_LIMIT}-byte limit; showing the kept tail\n"
        );
        text.push_str(&kept_text(&self.stderr));
        text
    }
}

/// Run the command with a null stdin and piped output streams.
///
/// Both readers run on their own threads, so neither pipe fills while the
/// child runs. The caller's own stdio settings are replaced. The wait loop
/// checks `token` every [`CHILD_POLL_INTERVAL`], stops the child when the
/// token is cancelled, and joins both readers before it returns. No child or
/// reader thread outlives the call.
pub(crate) fn run(mut command: Command, token: &CancellationToken) -> Result<GitOutput, RunError> {
    if token.is_cancelled() {
        return Err(RunError::Cancelled);
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(RunError::Io)?;
    let stdout = child
        .stdout
        .take()
        .expect("GitOutput invariant: stdout is piped");
    let stderr = child
        .stderr
        .take()
        .expect("GitOutput invariant: stderr is piped");
    let reader = thread::spawn(move || drain(stdout, false));
    let error_reader = thread::spawn(move || drain(stderr, true));
    let status = wait_for_child(&mut child, token);
    let reader = reader
        .join()
        .expect("GitOutput invariant: the stdout reader does not panic");
    let error_reader = error_reader
        .join()
        .expect("GitOutput invariant: the stderr reader does not panic");
    Ok(GitOutput {
        status: status?,
        stdout: reader.tail.bytes,
        stderr: error_reader.tail.bytes,
        truncated: reader.tail.truncated || error_reader.tail.truncated,
        low_speed: error_reader.low_speed,
    })
}

/// Wait for the child, and stop it when `token` is cancelled.
fn wait_for_child(child: &mut Child, token: &CancellationToken) -> Result<ExitStatus, RunError> {
    loop {
        if token.is_cancelled() {
            stop_child(child);
            return Err(RunError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => thread::sleep(CHILD_POLL_INTERVAL),
            Err(error) => {
                stop_child(child);
                return Err(RunError::Io(error));
            }
        }
    }
}

/// Stop the Git child and reap it.
///
/// This is the one place that stops a running Git child. A later change
/// replaces it with a whole-process-tree stop (issue #190).
fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

struct Stream {
    tail: Tail,
    low_speed: bool,
}

/// Read one stream to its end, keeping the bounded tail and the markers.
fn drain(mut stream: impl Read, scan_markers: bool) -> Stream {
    let mut tail = Tail::default();
    let mut scan = LowSpeedScan::new();
    let mut buffer = [0u8; READ_CHUNK];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                tail.push(&buffer[..read]);
                if scan_markers {
                    scan.feed(&buffer[..read]);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    Stream {
        tail,
        low_speed: scan.seen,
    }
}

/// The last bytes of one stream, never more than [`OUTPUT_TAIL_LIMIT`].
#[derive(Default)]
struct Tail {
    bytes: Vec<u8>,
    truncated: bool,
}

impl Tail {
    fn push(&mut self, chunk: &[u8]) {
        if chunk.len() >= OUTPUT_TAIL_LIMIT {
            self.bytes.clear();
            self.bytes
                .extend_from_slice(&chunk[chunk.len() - OUTPUT_TAIL_LIMIT..]);
            self.truncated = true;
            return;
        }
        let total = self.bytes.len() + chunk.len();
        if total > OUTPUT_TAIL_LIMIT {
            self.bytes.drain(..total - OUTPUT_TAIL_LIMIT);
            self.truncated = true;
        }
        self.bytes.extend_from_slice(chunk);
    }
}

/// Find the low-speed marker in a stream of chunks, across chunk boundaries.
struct LowSpeedScan {
    carry: Vec<u8>,
    seen: bool,
}

impl LowSpeedScan {
    fn new() -> Self {
        Self {
            carry: Vec::new(),
            seen: false,
        }
    }

    fn feed(&mut self, chunk: &[u8]) {
        if self.seen {
            return;
        }
        let mut probe = std::mem::take(&mut self.carry);
        probe.extend_from_slice(chunk);
        let lowered = probe.to_ascii_lowercase();
        if LOW_SPEED_MARKERS
            .iter()
            .any(|marker| contains(&lowered, marker))
        {
            self.seen = true;
            return;
        }
        let start = lowered.len().saturating_sub(MARKER_CARRY);
        self.carry = lowered[start..].to_vec();
    }
}

fn contains(text: &[u8], marker: &[u8]) -> bool {
    text.windows(marker.len()).any(|window| window == marker)
}

/// Decode kept bytes that may start inside a character, cut at the limit.
fn kept_text(bytes: &[u8]) -> String {
    let start = bytes
        .iter()
        .position(|byte| byte & 0xc0 != 0x80)
        .unwrap_or(bytes.len());
    let text = String::from_utf8_lossy(&bytes[start..]).into_owned();
    if text.len() <= OUTPUT_TAIL_LIMIT {
        return text;
    }
    let mut end = OUTPUT_TAIL_LIMIT;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

#[cfg(test)]
mod tests;
