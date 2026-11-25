#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommittedOutcome {
    Refused,
    OutputLost,
    StreamLost,
    Readable,
}

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
