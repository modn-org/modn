use crate::config::Config;
use crate::progress::Reporter;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::ffi::OsString;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

pub const OUTPUT_LIMIT: usize = 256 * 1024;

/// Reads newline-delimited backend output without losing a partially read line
/// when another `select!` branch wins. Consumed bytes stay in `buffer` until a
/// newline or EOF completes the line.
struct LineReader<R> {
    reader: R,
    buffer: Vec<u8>,
    eof: bool,
}

impl<R: AsyncBufRead + Unpin> LineReader<R> {
    fn new(reader: R) -> Self {
        Self {
            reader,
            buffer: Vec::new(),
            eof: false,
        }
    }

    /// Returns the next line without its trailing newline, or `None` at EOF.
    async fn next_line(&mut self) -> Result<Option<String>> {
        if self.eof {
            return Ok(None);
        }
        loop {
            let available = self.reader.fill_buf().await?;
            if available.is_empty() {
                self.eof = true;
                if self.buffer.is_empty() {
                    return Ok(None);
                }
                return Ok(Some(self.take_line()));
            }
            if let Some(index) = available.iter().position(|byte| *byte == b'\n') {
                self.buffer.extend_from_slice(&available[..index]);
                self.reader.consume(index + 1);
                return Ok(Some(self.take_line()));
            }
            let length = available.len();
            self.buffer.extend_from_slice(available);
            self.reader.consume(length);
            if self.buffer.len() > OUTPUT_LIMIT {
                bail!("backend output exceeds limit");
            }
        }
    }

    fn take_line(&mut self) -> String {
        let mut bytes = std::mem::take(&mut self.buffer);
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

async fn bounded_read(reader: impl AsyncRead + Unpin) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(OUTPUT_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > OUTPUT_LIMIT {
        bail!("backend output exceeds limit");
    }
    Ok(bytes)
}

/// The end of one backend call and what it reported about its commit point.
pub struct Completion {
    pub result: Result<Value>,
    pub committed: bool,
    pub cancelled: bool,
}

/// Channels between the running call and the stdio loop.
pub struct Channels {
    pub notifications: mpsc::UnboundedSender<Value>,
    pub cancellation: mpsc::UnboundedReceiver<()>,
}

/// Runs the CLI and forwards its progress events while it runs.
///
/// The CLI always receives `--progress json`. A cancellation signal writes one
/// `cancel` line to its stdin; the CLI ignores that line after its commit
/// point, which this function tracks.
pub async fn run(
    config: &Config,
    arguments: &[OsString],
    token: Option<Value>,
    channels: Channels,
) -> Completion {
    supervised(config, arguments, token, channels)
        .await
        .unwrap_or_else(|error| Completion {
            result: Err(error),
            committed: false,
            cancelled: false,
        })
}

async fn supervised(
    config: &Config,
    arguments: &[OsString],
    token: Option<Value>,
    channels: Channels,
) -> Result<Completion> {
    config.check_state()?;
    let parent = config.state.parent().context("state parent missing")?;
    let mut command = Command::new(&config.backend);
    command
        .args(["--json", "--progress", "json", "--state-dir"])
        .arg(&config.state)
        .args(arguments)
        .env_clear()
        .env("LOCALAPPDATA", parent)
        .env("XDG_STATE_HOME", parent)
        .current_dir(&config.state)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", system_root);
    }
    let mut child = command.spawn().context("backend could not start")?;
    let mut stdin = child.stdin.take().context("backend stdin missing")?;
    let stdout = child.stdout.take().context("backend stdout missing")?;
    let stderr = child.stderr.take().context("backend stderr missing")?;
    let mut stderr = LineReader::new(BufReader::new(stderr));
    let mut reporter = Reporter::new(token);
    let notifications = channels.notifications;
    let mut cancellation = channels.cancellation;
    let stdout = bounded_read(stdout);
    tokio::pin!(stdout);
    let mut output: Option<Result<Vec<u8>>> = None;
    let mut status: Option<ExitStatus> = None;
    let mut stdin_open = true;
    let mut stderr_eof = false;
    let mut stderr_bytes = 0usize;
    let mut failure: Option<anyhow::Error> = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(config.timeout_seconds);
    loop {
        if output.is_some() && status.is_some() && stderr_eof {
            break;
        }
        tokio::select! {
            result = &mut stdout, if output.is_none() => match result {
                Ok(output_bytes) => output = Some(Ok(output_bytes)),
                Err(error) => {
                    // Stop immediately: the unread stdout pipe would otherwise
                    // block the child until the deadline.
                    failure = Some(error);
                    break;
                }
            },
            line = stderr.next_line(), if !stderr_eof => match line {
                Ok(None) => stderr_eof = true,
                Ok(Some(line)) => {
                    if !Reporter::is_event(&line) {
                        stderr_bytes += line.len() + 1;
                        if stderr_bytes > OUTPUT_LIMIT {
                            failure = Some(anyhow!("backend output exceeds limit"));
                            break;
                        }
                    }
                    if let Some(notification) = reporter.event(&line) {
                        let _ = notifications.send(notification);
                    }
                }
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            },
            signal = cancellation.recv(), if stdin_open => {
                stdin_open = false;
                if signal.is_some() {
                    let _ = stdin.write_all(b"cancel\n").await;
                    let _ = stdin.flush().await;
                }
            }
            result = child.wait(), if status.is_none() => status = Some(result?),
            () = tokio::time::sleep_until(deadline) => {
                failure = Some(anyhow!("backend execution timed out"));
                break;
            }
        }
    }
    if let Some(error) = failure {
        terminate(&mut child).await;
        return Err(error);
    }
    let Some(status) = status else {
        terminate(&mut child).await;
        return Err(anyhow!("backend did not report an exit status"));
    };
    let output = match output.context("backend stdout missing")? {
        Ok(output) => output,
        Err(error) => {
            terminate(&mut child).await;
            return Err(error);
        }
    };
    let result = if status.success() {
        serde_json::from_slice(&output).context("backend returned invalid JSON")
    } else {
        Err(anyhow!(
            "backend failed with exit {}",
            status.code().unwrap_or(-1)
        ))
    };
    Ok(Completion {
        result,
        committed: reporter.is_committed(),
        cancelled: reporter.is_cancelled(),
    })
}

async fn terminate(child: &mut tokio::process::Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}
