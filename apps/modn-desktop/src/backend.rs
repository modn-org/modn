#[path = "backend/error.rs"]
pub(crate) mod error;
#[path = "backend/inactivity.rs"]
mod inactivity;
#[path = "backend/progress.rs"]
mod progress;

pub(crate) use error::{BackendError, BackendErrorKind};
pub(crate) use progress::{CancelToken, ProgressEvent};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

pub(crate) const OUTPUT_LIMIT: u64 = 1024 * 1024;
const ARCHIVE_LIMIT: u64 = 4 * 1024 * 1024 * 1024;
static SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub(crate) struct Backend {
    pub binary: PathBuf,
    pub state: PathBuf,
    pub data: Option<PathBuf>,
    pub modules: Option<PathBuf>,
    pub evidence: Option<PathBuf>,
    /// Inactivity timeout for the write and long-running call paths.
    ///
    /// `None` uses 120 seconds. The timeout applies only before the CLI reports
    /// its commit phase or starts its result on stdout.
    pub inactivity_timeout: Option<Duration>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct Game {
    pub game_id: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
pub(crate) struct Review {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Only the backend contract test's install helper reads this; the desktop commits through call_with_progress"
        )
    )]
    pub game: Game,
    pub archive: PathBuf,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Only the backend contract test's install helper reads this; the desktop commits through call_with_progress"
        )
    )]
    pub digest: Vec<u8>,
    pub plan: Value,
}

impl Backend {
    pub fn from_args() -> Result<Self> {
        let mut args = std::env::args_os().skip(1);
        let mut result = Self {
            binary: PathBuf::new(),
            state: PathBuf::new(),
            data: None,
            modules: None,
            evidence: None,
            inactivity_timeout: None,
        };
        while let Some(flag) = args.next() {
            let value = PathBuf::from(
                args.next()
                    .context("Each option requires an absolute path")?,
            );
            if !value.is_absolute() {
                bail!("Configuration paths must be absolute");
            }
            match flag.to_str() {
                Some("--backend") => result.binary = fs::canonicalize(value)?,
                Some("--state-dir") => result.state = value,
                Some("--data-dir") => result.data = Some(value),
                Some("--module-dir") => result.modules = Some(fs::canonicalize(value)?),
                Some("--evidence-dir") => result.evidence = Some(value),
                _ => bail!(
                    "Unknown option. Use --backend EXE --state-dir DIR; optional --data-dir, --module-dir, --evidence-dir"
                ),
            }
        }
        if !result.binary.is_file() || !result.state.is_absolute() {
            bail!(
                "Launch with --backend <absolute modn.exe> --state-dir <absolute manager directory>"
            );
        }
        #[cfg(windows)]
        if !result
            .binary
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
        {
            bail!("The fixed Windows backend must be an .exe file");
        }
        Ok(result)
    }

    /// Sets the inactivity timeout for the write and long-running call paths.
    ///
    /// The default is 120 seconds. The timeout applies only before the CLI
    /// reports the commit phase or starts its result on stdout; after that the
    /// call waits for the CLI to finish or roll back.
    #[must_use]
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Only the backend contract test sets a timeout; the desktop uses the default"
        )
    )]
    pub fn with_inactivity_timeout(mut self, timeout: Duration) -> Self {
        self.inactivity_timeout = Some(timeout);
        self
    }

    /// Runs a write command with `--progress json` and cooperative cancellation.
    ///
    /// Each stderr JSON object with a `phase` field is passed to `on_event` in
    /// the order the CLI wrote it. The final JSON result on stdout is returned.
    /// When `cancel` is cancelled, the call writes one `cancel` line on stdin
    /// and never kills the process for that reason.
    ///
    /// Before the commit event, inactivity for the configured timeout sends
    /// `cancel`, waits 15 seconds and then kills the process. The deadline
    /// stays armed after a cancel, so a step that stalls instead of stopping
    /// cannot block the call or window close. After the commit event there is
    /// no inactivity kill, because a committed write must be allowed to finish
    /// or roll back.
    ///
    /// Output that exceeds the bound before the commit event fails the call.
    /// Output that exceeds it after the commit event is drained, the writer is
    /// not killed, and the error code is
    /// `BACKEND_OUTPUT_TRUNCATED_AFTER_COMMIT`: the files are installed, so
    /// callers refresh instead of reporting a failed write.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError`] when the CLI refuses, the operation is
    /// cancelled before the commit point, output is invalid or exceeds its
    /// bound before the commit point, the process cannot start, or inactivity
    /// expires before the commit point.
    pub fn call_with_progress(
        &self,
        args: &[OsString],
        cancel: &CancelToken,
        on_event: &mut dyn FnMut(ProgressEvent),
    ) -> Result<Value, BackendError> {
        progress::call(self, args, cancel, on_event)
    }

    /// Runs a command that does not report progress with the inactivity
    /// timeout.
    ///
    /// `--progress json` is not appended. Any output resets the timer. The
    /// call ends when the CLI process exits, so a started game that inherited
    /// the CLI's pipes cannot hold the caller open. Once stdout carries output,
    /// the call never kills the process: the write has committed and its
    /// result is recoverable.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError`] when the CLI refuses, output is invalid or
    /// exceeds its bound before stdout starts, the process cannot start, or
    /// inactivity expires before stdout starts.
    pub fn call_with_inactivity_timeout(&self, args: &[OsString]) -> Result<Value, BackendError> {
        inactivity::call(self, args)
    }

    pub fn call(&self, args: &[OsString]) -> Result<Value> {
        let mut child = progress::spawn(self, args, Stdio::null())
            .context("Cannot start configured backend. Check its path and permissions")?;
        let stdout = child.stdout.take().context("Missing backend stdout")?;
        let stderr = child.stderr.take().context("Missing backend stderr")?;
        let (tx, rx) = mpsc::channel();
        for (is_error, pipe) in [
            (false, Box::new(stdout) as Box<dyn Read + Send>),
            (true, Box::new(stderr)),
        ] {
            let tx = tx.clone();
            thread::spawn(move || {
                let mut bytes = Vec::new();
                let read = pipe.take(OUTPUT_LIMIT + 1).read_to_end(&mut bytes);
                let _ = tx.send((is_error, bytes, read));
            });
        }
        drop(tx);
        let start = Instant::now();
        let mut streams = [None, None];
        let status = loop {
            while let Ok((error, bytes, read)) = rx.try_recv() {
                if read.is_err() || bytes.len() as u64 > OUTPUT_LIMIT {
                    let _ = child.kill();
                    let _ = child.wait();
                    bail!(
                        "Backend output exceeded its bound or could not be read. Check CLI doctor and recover before retrying writes"
                    );
                }
                streams[usize::from(error)] = Some(bytes);
            }
            match child.try_wait() {
                Ok(Some(status)) if streams.iter().all(Option::is_some) => break status,
                Ok(_) => {}
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error.into());
                }
            }
            if start.elapsed() > Duration::from_secs(60) {
                let _ = child.kill();
                let _ = child.wait();
                bail!(
                    "Backend exceeded 60 seconds. A write may need recovery. Run CLI doctor and recover before retrying"
                );
            }
            thread::sleep(Duration::from_millis(20));
        };
        let stdout = String::from_utf8_lossy(streams[0].as_deref().unwrap_or_default());
        let stderr = String::from_utf8_lossy(streams[1].as_deref().unwrap_or_default());
        if let Some(folder) = &self.evidence {
            fs::create_dir_all(folder).context("Backend finished, but evidence storage failed. A write may have completed; check CLI inventory and doctor")?;
            let record = serde_json::json!({"binary": self.binary, "args": args.iter().map(|arg| arg.to_string_lossy()).collect::<Vec<_>>(), "exit": status.code(), "stdout": stdout, "stderr": stderr});
            fs::write(
                folder.join(format!(
                    "backend-{:04}.json",
                    SEQUENCE.fetch_add(1, Ordering::Relaxed)
                )),
                serde_json::to_vec_pretty(&record)?,
            ).context("Backend finished, but evidence recording failed. A write may have completed; check CLI inventory and doctor")?;
        }
        if !status.success() {
            bail!(
                "Backend refused (exit {status}): {stderr}\n{stdout}\nResolve this in the CLI, then review again."
            );
        }
        serde_json::from_str(&stdout)
            .context("Backend returned invalid JSON; verify the configured CLI version")
    }

    pub fn games(&self) -> Result<Vec<Game>> {
        Ok(serde_json::from_value(
            self.call(&["game".into(), "list".into()])?,
        )?)
    }

    fn selected(&self, game: &Game) -> Result<Vec<OsString>> {
        if !self.games()?.contains(game) {
            bail!("Game registration changed. Refresh and select the installation again");
        }
        Ok(vec!["--game".into(), game.game_id.clone().into()])
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Only the backend contract test lists owned packages through this helper; the desktop reads library state through call"
        )
    )]
    pub fn read(&self, game: &Game, action: &str) -> Result<Value> {
        if !["list", "doctor"].contains(&action) {
            bail!("Unsupported read action");
        }
        let mut args = self.selected(game)?;
        args.extend([
            action.into(),
            "--path".into(),
            game.path.clone().into_os_string(),
        ]);
        self.call(&args)
    }

    fn preview(&self, game: &Game, archive: &Path) -> Result<Value> {
        let mut args = self.selected(game)?;
        args.extend([
            "install".into(),
            archive.as_os_str().into(),
            "--dry-run".into(),
            "--path".into(),
            game.path.clone().into_os_string(),
        ]);
        self.call(&args)
    }

    pub fn review(&self, game: &Game, source: &Path) -> Result<Review> {
        let archive =
            fs::canonicalize(source).context("Choose an existing local ZIP or 7z archive")?;
        let extension = archive
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or_default();
        if !["zip", "7z"].contains(&extension.to_ascii_lowercase().as_str()) {
            bail!("Choose a local ZIP or 7z archive");
        }
        let digest = digest(&archive)?;
        self.call(&["inspect".into(), archive.clone().into_os_string()])?;
        let plan = self.preview(game, &archive)?;
        if digest != self::digest(&archive)? {
            bail!("Archive changed during review. Review again");
        }
        Ok(Review {
            game: game.clone(),
            archive,
            digest,
            plan,
        })
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Only the backend contract test uses the one-shot install helper; the desktop commits through call_with_progress"
        )
    )]
    pub fn install(&self, review: &Review) -> Result<Value> {
        if !reviewable(&review.plan) {
            bail!("Plan is blocked. Resolve blockers and review again");
        }
        if digest(&review.archive)? != review.digest {
            bail!("Archive changed since review. Review again");
        }
        let fresh = self.preview(&review.game, &review.archive)?;
        if fresh != review.plan || digest(&review.archive)? != review.digest {
            bail!("Installation plan or archive changed. Review again");
        }
        let mut args = self.selected(&review.game)?;
        args.extend([
            "install".into(),
            review.archive.clone().into_os_string(),
            "--yes".into(),
            "--path".into(),
            review.game.path.clone().into_os_string(),
        ]);
        self.call(&args)
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Only the backend contract test uses the one-shot uninstall helper; the desktop commits through call_with_progress"
        )
    )]
    pub fn uninstall(&self, game: &Game, id: &str) -> Result<Value> {
        let packages = self.read(game, "list")?;
        if !packages
            .as_array()
            .is_some_and(|items| items.iter().any(|p| p["id"].as_str() == Some(id)))
        {
            bail!("Package selection changed. Refresh owned packages");
        }
        let mut args = self.selected(game)?;
        args.extend([
            "uninstall".into(),
            id.into(),
            "--yes".into(),
            "--path".into(),
            game.path.clone().into_os_string(),
        ]);
        self.call(&args)
    }
}

fn digest(path: &Path) -> Result<Vec<u8>> {
    let mut file = fs::File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > ARCHIVE_LIMIT {
        bail!("Choose a regular archive file no larger than 4 GiB");
    }
    let mut total = 0_u64;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > ARCHIVE_LIMIT {
            bail!("Archive grew beyond the 4 GiB review limit");
        }
        hash.update(&buffer[..count]);
    }
    Ok(hash.finalize().to_vec())
}

pub(crate) fn reviewable(plan: &Value) -> bool {
    plan["status"] == "reviewable"
        && plan["metadata_only"] == true
        && plan["blockers"].as_array().is_some_and(Vec::is_empty)
        && plan["operations"].as_array().is_some_and(|ops| {
            !ops.is_empty()
                && ops.iter().all(|op| {
                    ["source", "destination"].iter().all(|key| {
                        op[*key]
                            .as_str()
                            .is_some_and(|value| !value.trim().is_empty())
                    })
                })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_backend() -> Backend {
        Backend {
            binary: PathBuf::from(r"C:\modn-test\missing-backend.exe"),
            state: PathBuf::from(r"C:\modn-test\state"),
            data: None,
            modules: None,
            evidence: None,
            inactivity_timeout: None,
        }
    }

    fn test_game() -> Game {
        Game {
            game_id: "fixture".into(),
            path: PathBuf::from(r"C:\modn-test\game"),
        }
    }

    #[test]
    fn install_refuses_a_blocked_plan_before_reading_the_archive() {
        let review = Review {
            game: test_game(),
            archive: PathBuf::from(r"C:\modn-test\missing.zip"),
            digest: Vec::new(),
            plan: serde_json::json!({"status": "blocked"}),
        };
        assert!(test_backend().install(&review).is_err());
    }

    #[test]
    fn games_and_uninstall_fail_closed_when_the_backend_is_missing() {
        let backend = test_backend();
        assert!(backend.games().is_err());
        assert!(backend.uninstall(&test_game(), "package").is_err());
    }

    #[test]
    fn inactivity_timeout_is_recorded() {
        let backend = test_backend().with_inactivity_timeout(Duration::from_millis(5));
        assert_eq!(backend.inactivity_timeout, Some(Duration::from_millis(5)));
    }

    #[test]
    fn only_complete_operations_in_unblocked_metadata_plans_enable_install() {
        let mut plan = serde_json::json!({"status":"reviewable", "metadata_only":true, "blockers":[], "operations":[{"source":"plugins/demo.plugin", "destination":"addons/demo.plugin"}]});
        assert!(reviewable(&plan));
        for operations in [
            serde_json::json!([{}]),
            serde_json::json!([null]),
            serde_json::json!([{"source":"plugins/demo.plugin"}]),
            serde_json::json!([{"destination":"addons/demo.plugin"}]),
            serde_json::json!([{"source":1, "destination":"addons/demo.plugin"}]),
            serde_json::json!([{"source":"plugins/demo.plugin", "destination":null}]),
            serde_json::json!([{"source":"", "destination":"addons/demo.plugin"}]),
            serde_json::json!([{"source":"plugins/demo.plugin", "destination":" "}]),
            serde_json::json!([{"source":"plugins/demo.plugin", "destination":"addons/demo.plugin"}, {}]),
        ] {
            let mut malformed = plan.clone();
            malformed["operations"] = operations;
            assert!(!reviewable(&malformed), "malformed operations: {malformed}");
        }
        for blocked in [
            serde_json::json!([{"message":"approval required"}]),
            Value::Null,
        ] {
            plan["blockers"] = blocked;
            assert!(!reviewable(&plan));
        }
        assert!(!reviewable(&serde_json::json!({})));
    }
}
