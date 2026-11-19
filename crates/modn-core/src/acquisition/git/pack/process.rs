//! One Git fetch child: its progress output, its stderr and its process tree.

use super::super::super::READ_INACTIVITY_TIMEOUT;
use super::registry::{self, Registration};
use crate::git::{self, GitEnvironment};
use anyhow::{Context, Result};
use std::ffi::OsStr;
use std::fmt;
use std::io::{self, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How often the timeout path checks whether the child exited.
const KILL_POLL: Duration = Duration::from_millis(25);

/// A fetch tree that did not end.
///
/// When the tree cannot be stopped, the caller must report a hard error and
/// must not fall over to another source: a surviving descendant could still
/// write into the staging repository.
#[derive(Debug)]
pub(crate) struct TreeStillRunning {
    reason: String,
}

impl TreeStillRunning {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl fmt::Display for TreeStillRunning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the Git fetch process tree could not be stopped: {}",
            self.reason
        )
    }
}

impl std::error::Error for TreeStillRunning {}

/// Whether `error` reports a fetch tree that could not be stopped.
pub(crate) fn is_tree_still_running(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.downcast_ref::<TreeStillRunning>().is_some())
}

/// Whether a caller may end the tree, or a test must observe the failure path.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TreeStop {
    /// End the tree and report whether it ended.
    End,
    /// A seam that reports a failure after the real attempt, for tests only.
    #[cfg(any(test, feature = "test-instruction-budget"))]
    Fail,
}

/// A running Git command with its stderr reader and its progress counter.
///
/// The reader counts every byte Git writes to stderr. A command that keeps
/// writing progress output therefore counts as alive even when its total run
/// time exceeds the inactivity limit.
pub(super) struct FetchProcess {
    child: Child,
    progress: Arc<AtomicU64>,
    text: Receiver<String>,
    reader: Option<JoinHandle<()>>,
    registration: Registration,
}

impl FetchProcess {
    /// Start one hardened Git fetch, with a replacement program in test builds.
    pub(super) fn spawn(
        environment: &GitEnvironment<'_>,
        url: &str,
        commit: &str,
        program: Option<&OsStr>,
    ) -> Result<Self> {
        let mut command = git::command(environment);
        command
            .arg("-c")
            .arg("http.lowSpeedLimit=1")
            .arg("-c")
            .arg(format!(
                "http.lowSpeedTime={}",
                READ_INACTIVITY_TIMEOUT.as_secs()
            ))
            .args([
                "fetch",
                "--progress",
                "--depth",
                "1",
                "--no-tags",
                "--",
                url,
                commit,
            ]);
        Self::start(command, program)
    }

    /// Start one hardened Git command with explicit arguments.
    pub(super) fn spawn_arguments(
        environment: &GitEnvironment<'_>,
        arguments: &[&OsStr],
        program: Option<&OsStr>,
    ) -> Result<Self> {
        let mut command = git::command(environment);
        command
            .arg("-c")
            .arg("http.lowSpeedLimit=1")
            .arg("-c")
            .arg(format!(
                "http.lowSpeedTime={}",
                READ_INACTIVITY_TIMEOUT.as_secs()
            ))
            .args(arguments);
        Self::start(command, program)
    }

    fn start(mut command: Command, program: Option<&OsStr>) -> Result<Self> {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        #[cfg(any(test, feature = "test-instruction-budget"))]
        let mut command = match program {
            Some(program) => replace_program(command, program),
            None => command,
        };
        #[cfg(not(any(test, feature = "test-instruction-budget")))]
        let _ = program;
        in_own_group(&mut command);
        let mut child = command.spawn().context(
            "GIT_SOURCE_FETCH_FAILED: cannot run Git; install Git and ensure it is on PATH",
        )?;
        let registration = registry::register(child.id());
        let mut stderr = child
            .stderr
            .take()
            .context("GIT_SOURCE_FETCH_FAILED: git fetch stderr is not captured")?;
        let progress = Arc::new(AtomicU64::new(0));
        let seen = Arc::clone(&progress);
        let (sender, text) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut collected = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                match stderr.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        seen.fetch_add(read as u64, Ordering::Relaxed);
                        collected.extend_from_slice(&buffer[..read]);
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
            let _ = sender.send(String::from_utf8_lossy(&collected).into_owned());
        });
        Ok(Self {
            child,
            progress,
            text,
            reader: Some(reader),
            registration,
        })
    }

    /// Bytes Git has written to stderr since the command started.
    pub(super) fn progress(&self) -> u64 {
        self.progress.load(Ordering::Relaxed)
    }

    /// Report the exit status once the child ends.
    pub(super) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    /// Stop the child and every descendant, then reap the child within `grace`.
    ///
    /// # Errors
    ///
    /// Returns [`TreeStillRunning`] when the tree could not be ended or the
    /// child was not reaped within `grace`. The caller must then stop instead
    /// of trying another source.
    pub(super) fn terminate(
        &mut self,
        grace: Duration,
        stop: TreeStop,
    ) -> Result<(), TreeStillRunning> {
        if let Err(error) = self.end_tree() {
            // The tree may still run, so an interrupt handler must try again.
            self.registration.keep();
            return Err(error);
        }
        #[cfg(any(test, feature = "test-instruction-budget"))]
        if stop == TreeStop::Fail {
            // The seam reports a failure after the real tree ended, so this
            // registration can end with the fetch.
            return Err(TreeStillRunning::new(
                "the test seam cannot confirm the end of the tree",
            ));
        }
        let _ = stop;
        let _ = self.child.kill();
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => return Ok(()),
                Err(error) => {
                    // A tree that cannot be reaped may still run.
                    self.registration.keep();
                    return Err(TreeStillRunning::new(format!(
                        "cannot reap the fetch process: {error}"
                    )));
                }
                Ok(None) => std::thread::sleep(KILL_POLL),
            }
        }
        // The child did not end, so the tree stays registered for a retry.
        self.registration.keep();
        Err(TreeStillRunning::new(format!(
            "the fetch process did not end within {} ms",
            grace.as_millis()
        )))
    }

    /// The child's stderr text, once the reader ends within `grace`.
    ///
    /// A read that does not end within the grace leaves the reader detached,
    /// so a descendant outside the tree can never block the caller.
    pub(super) fn stderr(&mut self, grace: Duration) -> String {
        let Ok(text) = self.text.recv_timeout(grace) else {
            return String::new();
        };
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        text
    }

    /// The child's stderr text after its tree ended.
    ///
    /// Every writer of the pipe is gone, so the reader must see the end of the
    /// stream. The caller joins it instead of detaching it.
    pub(super) fn stderr_after_tree(&mut self) -> String {
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        self.text.try_recv().unwrap_or_default()
    }

    #[cfg(windows)]
    fn end_tree(&mut self) -> Result<(), TreeStillRunning> {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        use std::os::windows::process::CommandExt;
        let pid = self.child.id().to_string();
        let mut taskkill = Command::new("taskkill");
        taskkill
            .args(["/T", "/F", "/PID", &pid])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW);
        match taskkill.status() {
            Ok(status) if status.success() => Ok(()),
            // 128 means no process with that PID exists. The child ended
            // between the last poll and the kill, so no tree is left to stop.
            Ok(status) if status.code() == Some(128) => Ok(()),
            Ok(status) => Err(TreeStillRunning::new(format!(
                "taskkill /T /F /PID {pid} ended with {status}"
            ))),
            Err(error) => Err(TreeStillRunning::new(format!(
                "taskkill /T /F /PID {pid} could not run: {error}"
            ))),
        }
    }

    #[cfg(unix)]
    fn end_tree(&mut self) -> Result<(), TreeStillRunning> {
        let pid = i32::try_from(self.child.id()).map_err(|_| {
            TreeStillRunning::new("the fetch process id does not fit a process group id")
        })?;
        let Some(group) = rustix::process::Pid::from_raw(pid) else {
            return Err(TreeStillRunning::new(
                "the fetch process id is not a valid process group id",
            ));
        };
        match rustix::process::kill_process_group(group, rustix::process::Signal::KILL) {
            // `SRCH` means the group is already gone.
            Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
            Err(error) => Err(TreeStillRunning::new(format!(
                "cannot signal the fetch process group: {error}"
            ))),
        }
    }
}

/// Start the child in its own process group so a timeout can end the tree.
#[cfg(unix)]
fn in_own_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

/// Windows ends the tree through `taskkill`; no spawn setting is needed.
#[cfg(windows)]
fn in_own_group(_command: &mut Command) {}

/// Reuse a prepared Git command with a test program in place of `git`.
#[cfg(any(test, feature = "test-instruction-budget"))]
fn replace_program(command: Command, program: &OsStr) -> Command {
    let mut replaced = Command::new(program);
    replaced
        .args(command.get_args())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if let Some(directory) = command.get_current_dir() {
        replaced.current_dir(directory);
    }
    for (key, value) in command.get_envs() {
        match value {
            Some(value) => {
                replaced.env(key, value);
            }
            None => {
                replaced.env_remove(key);
            }
        }
    }
    replaced
}
