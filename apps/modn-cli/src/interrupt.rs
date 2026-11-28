//! One interrupt path for every command.
//!
//! A terminal Ctrl-C used to reach Git directly, because Git shared Modn's
//! foreground process group. Git now leads its own group, so Modn cancels its
//! own work instead. The handler only cancels the process-wide token; the Git
//! runner turns that into the tree stop. A command that does not consume the
//! token would otherwise ignore the interrupt, so the handler ends the process
//! after a bounded wait.

use anyhow::{Context, Result};
use modn_core::progress::interrupt_token;
use std::time::Duration;

/// How long the handler waits for the command to end after a cancellation.
const BACKSTOP: Duration = Duration::from_secs(5);

/// Register the one handler for Ctrl-C, SIGTERM/SIGHUP and a console break event.
pub(super) fn install() -> Result<()> {
    ctrlc::set_handler(|| {
        on_interrupt();
        backstop();
    })
    .context("cannot register the interrupt handler")
}

/// The handler action: cancel the process-wide token.
fn on_interrupt() {
    interrupt_token().cancel();
}

/// End the process when no operation consumed the interrupt.
fn backstop() -> ! {
    std::thread::sleep(BACKSTOP);
    std::process::exit(exit_code())
}

/// The conventional interrupt status: 128 plus SIGINT on Unix, 1 on Windows.
fn exit_code() -> i32 {
    if cfg!(windows) {
        1
    } else {
        130
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The token is process-wide, so it stays cancelled for the rest of the
    /// CLI unit-test process. No CLI unit test starts a Git runner that reads
    /// it.
    #[test]
    fn the_interrupt_handler_cancels_the_process_token() {
        assert!(!interrupt_token().is_cancelled());
        on_interrupt();
        assert!(interrupt_token().is_cancelled());
    }
}
