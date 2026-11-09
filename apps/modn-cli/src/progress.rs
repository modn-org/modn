use anyhow::Result;
use clap::ValueEnum;
use modn_core::error::ErrorCode;
use modn_core::progress::{CancellationToken, OperationControl, Outcome};
use std::io::{self, BufRead};

/// Output format of `--progress`.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub(super) enum ProgressFormat {
    /// One JSON event per line on stderr.
    Json,
}

/// Progress and cancellation for one managed operation.
///
/// Disabled, it reports nothing and never cancels. Enabled with
/// `--progress json`, it writes one JSON event per line to stderr and a
/// background reader cancels the operation on a `cancel` line on stdin.
pub(super) struct Progress {
    control: OperationControl,
    enabled: bool,
}

impl Progress {
    pub(super) fn start(format: Option<ProgressFormat>) -> Self {
        let enabled = format.is_some();
        if !enabled {
            return Self {
                control: OperationControl::default(),
                enabled,
            };
        }
        let token = CancellationToken::new();
        let cancel = token.clone();
        std::thread::spawn(move || {
            for line in io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                if line.trim() == "cancel" {
                    cancel.cancel();
                }
            }
        });
        let control = OperationControl::new(token, |event| match serde_json::to_string(event) {
            Ok(line) => eprintln!("{line}"),
            Err(_) => {
                eprintln!(
                    "{{\"phase\":\"error\",\"message\":\"cannot serialize progress event\"}}"
                );
            }
        });
        Self { control, enabled }
    }

    pub(super) fn control(&self) -> &OperationControl {
        &self.control
    }

    pub(super) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(super) fn begin(&self) -> Result<()> {
        self.control.begin()?;
        Ok(())
    }

    pub(super) fn finish_success(&self) {
        self.control.finish(Outcome::Succeeded, None);
    }

    pub(super) fn finish_error(&self, code: ErrorCode) {
        if code == ErrorCode::Cancelled {
            self.control.finish_cancelled();
        } else {
            self.control.finish_error(code);
        }
    }
}
