use crate::config::Config;
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::ffi::OsString;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

pub const OUTPUT_LIMIT: usize = 256 * 1024;

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

pub async fn run(config: &Config, arguments: &[OsString]) -> Result<Value> {
    config.check_state()?;
    let parent = config.state.parent().context("state parent missing")?;
    let mut command = Command::new(&config.backend);
    command
        .args(["--json", "--state-dir"])
        .arg(&config.state)
        .args(arguments)
        .env_clear()
        .env("LOCALAPPDATA", parent)
        .env("XDG_STATE_HOME", parent)
        .current_dir(&config.state)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", system_root);
    }
    let mut child = command.spawn().context("backend could not start")?;
    let stdout = child.stdout.take().context("backend stdout missing")?;
    let stderr = child.stderr.take().context("backend stderr missing")?;
    let outcome = tokio::time::timeout(Duration::from_secs(config.timeout_seconds), async {
        tokio::try_join!(bounded_read(stdout), bounded_read(stderr), async {
            Ok(child.wait().await?)
        })
    })
    .await;
    match outcome {
        Ok(Ok((output, _diagnostics, status))) => {
            if !status.success() {
                bail!("backend failed with exit {}", status.code().unwrap_or(-1));
            }
            serde_json::from_slice(&output).context("backend returned invalid JSON")
        }
        result => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            match result {
                Err(_) => bail!("backend execution timed out"),
                Ok(Err(error)) => Err(error),
                Ok(Ok(_)) => unreachable!("successful outcome handled above"),
            }
        }
    }
}
