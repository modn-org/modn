//! One Git fetch child: its progress output, its stderr and its process tree.

use super::super::super::READ_INACTIVITY_TIMEOUT;
use crate::git::{self, GitEnvironment};
use anyhow::{Context, Result};
use std::ffi::OsStr;
use std::io::{self, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How often the timeout path checks whether the child exited.
const KILL_POLL: Duration = Duration::from_millis(25);

/// A running Git fetch with its stderr reader and its progress counter.
///
/// The reader counts every byte Git writes to stderr. A fetch that keeps
/// writing progress output therefore counts as alive even when its total run
/// time exceeds the inactivity limit.
pub(super) struct FetchProcess {
    child: Child,
    progress: Arc<AtomicU64>,
    text: Receiver<String>,
    reader: Option<JoinHandle<()>>,
}

impl FetchProcess {
    /// Start one hardened Git fetch, with a replacement program in test builds.
    pub(super) fn spawn(
        environment: &GitEnvironment<'_>,
        url: &str,
        commit: &str,
        program: Option<&OsStr>,
    ) -> Result<Self> {
        let mut command = command(environment, url, commit, program);
        in_own_group(&mut command);
        let mut child = command.spawn().context(
            "GIT_SOURCE_FETCH_FAILED: cannot run Git; install Git and ensure it is on PATH",
        )?;
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
        })
    }

    /// Bytes Git has written to stderr since the fetch started.
    pub(super) fn progress(&self) -> u64 {
        self.progress.load(Ordering::Relaxed)
    }

    /// Report the exit status once the child ends.
    pub(super) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    /// Wait for the child and reap it.
    pub(super) fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait()
    }

    /// Stop the child and every descendant, then reap the child within `grace`.
    pub(super) fn terminate(&mut self, grace: Duration) {
        self.end_tree();
        let _ = self.child.kill();
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) | Err(_) => return,
                Ok(None) => std::thread::sleep(KILL_POLL),
            }
        }
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

    #[cfg(windows)]
    fn end_tree(&mut self) {
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
        let _ = taskkill.status();
    }

    #[cfg(unix)]
    fn end_tree(&mut self) {
        let Ok(pid) = i32::try_from(self.child.id()) else {
            return;
        };
        let Some(group) = rustix::process::Pid::from_raw(pid) else {
            return;
        };
        let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
    }
}

/// Build the fixed fetch command, without its process-group setting.
fn command(
    environment: &GitEnvironment<'_>,
    url: &str,
    commit: &str,
    program: Option<&OsStr>,
) -> Command {
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
            "fetch", "--progress", "--depth", "1", "--no-tags", "--", url, commit,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(any(test, feature = "test-instruction-budget"))]
    if let Some(program) = program {
        return replace_program(command, program);
    }
    #[cfg(not(any(test, feature = "test-instruction-budget")))]
    let _ = program;
    command
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
