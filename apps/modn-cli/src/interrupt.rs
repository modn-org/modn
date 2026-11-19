//! Forward a Unix interrupt to the live Git fetch process trees.
//!
//! A Git fetch runs in its own process group so the timeout path can end the
//! whole tree. A terminal Ctrl-C then reaches only Modn, so Modn ends every
//! live fetch tree itself. It exits with the status the default signal action
//! would report: 128 plus the signal number, so 130 for SIGINT and 143 for
//! SIGTERM.

use modn_core::acquisition::end_active_fetch_trees;
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

/// Install the interrupt handler thread.
///
/// A failure leaves Modn running with the previous signal behavior. The
/// function reports nothing, because the CLI has no output channel yet.
pub(crate) fn install() {
    let Ok(mut signals) = Signals::new([SIGINT, SIGTERM]) else {
        return;
    };
    let _ = std::thread::Builder::new()
        .name("modn-interrupt".to_owned())
        .spawn(move || {
            if let Some(signal) = signals.forever().next() {
                end_active_fetch_trees();
                std::process::exit(128 + signal);
            }
        });
}
