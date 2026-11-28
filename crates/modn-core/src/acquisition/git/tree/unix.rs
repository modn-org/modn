//! Unix process-group tree confinement.
//!
//! `CommandExt::process_group(0)` makes the Git child the leader of a new
//! process group. Every helper it starts inherits that group unless it moves
//! itself, which Git does not do for `git-remote-https`. A stop sends SIGKILL
//! to the negative group ID, so it reaches the leader and every helper.
//!
//! Group-ID reuse: the kernel keeps a process-group ID allocated while any
//! process is in the group. After the direct child was reaped, Modn checks the
//! group first and sends a signal only when that check still reports the group
//! as non-empty. The check and the signal are not one atomic operation, but the
//! signal is never sent for a group that this guard already observed empty, and
//! once a signal was sent the only later operation is the empty check.

use super::{wait_until_empty, TreeError};
use rustix::io::Errno;
use rustix::process::{getpgid, kill_process_group, test_kill_process_group, Pid, Signal};
use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command};

/// One process group that the Git child leads.
pub(crate) struct Tree {
    group: Pid,
}

impl Tree {
    /// Make the child the leader of a new process group.
    pub(crate) fn confine(command: &mut Command) -> io::Result<()> {
        command.process_group(0);
        Ok(())
    }

    /// Record the process group that the started child leads.
    pub(crate) fn attach(child: &Child) -> Result<Self, TreeError> {
        let pid = Pid::from_raw(child.id() as i32).ok_or_else(|| {
            TreeError::Io(io::Error::other(format!(
                "the Git child process ID {} is not a valid Unix process ID",
                child.id()
            )))
        })?;
        let group = getpgid(Some(pid)).map_err(|error| TreeError::Io(error.into()))?;
        if group != pid {
            return Err(TreeError::Io(io::Error::other(format!(
                "the Git child {} leads process group {group}, not its own group",
                child.id()
            ))));
        }
        Ok(Self { group })
    }

    /// Kill the group, reap the direct child and confirm it is empty.
    pub(crate) fn stop(&self, child: &mut Child) -> Result<(), TreeError> {
        self.kill_group()?;
        child.wait().map_err(TreeError::Io)?;
        self.confirm_empty()
    }

    /// Kill what is left after the direct child exited on its own.
    pub(crate) fn finish(&self) -> Result<(), TreeError> {
        match test_kill_process_group(self.group) {
            // The group is still allocated, so this ID cannot belong to a new
            // group. This is the only signal Modn sends after the leader was
            // reaped.
            Ok(()) => self.kill_group()?,
            Err(Errno::SRCH) => {}
            Err(error) => return Err(TreeError::Io(error.into())),
        }
        self.confirm_empty()
    }

    /// Kill the group without waiting. Only a dropping guard uses this.
    pub(crate) fn abandon(&self) {
        if matches!(test_kill_process_group(self.group), Ok(())) {
            let _ = kill_process_group(self.group, Signal::KILL);
        }
    }

    fn kill_group(&self) -> Result<(), TreeError> {
        match kill_process_group(self.group, Signal::KILL) {
            // The leader exited between the wait poll and this call.
            Err(Errno::SRCH) => Ok(()),
            result => result.map_err(|error| TreeError::Io(error.into())),
        }
    }

    fn confirm_empty(&self) -> Result<(), TreeError> {
        wait_until_empty(|| match test_kill_process_group(self.group) {
            Ok(()) => Ok(false),
            Err(Errno::SRCH) => Ok(true),
            Err(error) => Err(error.into()),
        })
    }
}
