//! The decision that ends a call once its result may have committed.
//!
//! The progress path learns the commit from the CLI's `commit` event and the
//! inactivity path from stdout starting. Both paths decide the same way: the
//! exit status comes first, and a lost result stream after a successful exit
//! is a committed outcome rather than plain success or a plain output failure.

/// The outcome to report once the CLI may have committed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommittedOutcome {
    /// The CLI exited with a failure, so the commit did not complete.
    Refused,
    /// The committed result could not be read.
    OutputLost,
    /// stderr could not be read, so the operation has no progress warning.
    StreamLost,
    /// stdout was read within its bound and still has to be parsed.
    Readable,
}

/// Decides how a call that may have committed ends.
///
/// The exit status comes first: a `commit` event or a started result stream
/// alone does not prove that the write completed, so a failed exit is a
/// refusal even when the result stream was lost. A lost stream with a
/// successful exit is a committed outcome that callers refresh, never plain
/// success. So is a readable stream whose bytes are not one JSON value: the
/// write is installed and only its result is unknown.
pub(crate) fn committed_outcome(
    exit_ok: bool,
    stdout_lost: bool,
    stderr_lost: bool,
) -> CommittedOutcome {
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
