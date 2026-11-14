//! The single process-spawn helper of the desktop backend.

use super::super::Backend;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;
use std::process::{Child, Command, Stdio};

/// Starts the configured CLI with the fixed argument prefix.
///
/// This is the only place in the crate that creates a [`Command`]. The child
/// environment is cleared, then only these variables are forwarded from the
/// app's own environment when they are set:
///
/// - `SystemRoot`: process basics on Windows.
/// - `TEMP` and `TMP`: only when the value is an absolute Windows path. A
///   POSIX value such as Git Bash's `/tmp`, or a drive-relative value, is left
///   out so the CLI keeps its default instead of resolving the path against
///   the current drive.
/// - `LOCALAPPDATA`: Modn's data directory, unless `--data-dir` overrides it.
/// - `MODN_CACHE`, `MODN_STORE_LIBRARIES` and `MODN_APPDATA`: disposable
///   fixture overrides.
/// - `MODN_GAME_MODULES_DIR`: the module source, unless `--module-dir`
///   overrides it.
pub(crate) fn spawn(backend: &Backend, args: &[OsString], stdin: Stdio) -> io::Result<Child> {
    let mut command = Command::new(&backend.binary);
    command.args([
        OsString::from("--json"),
        OsString::from("--state-dir"),
        backend.state.clone().into_os_string(),
    ]);
    command
        .args(args)
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.env_clear();
    for name in [
        "SystemRoot",
        "MODN_CACHE",
        "MODN_STORE_LIBRARIES",
        "MODN_APPDATA",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    for name in ["TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(name)
            && is_absolute_windows_path(&value)
        {
            command.env(name, value);
        }
    }
    if let Some(modules) = &backend.modules {
        command.env("MODN_GAME_MODULES_DIR", modules);
    } else if let Some(value) = std::env::var_os("MODN_GAME_MODULES_DIR") {
        command.env("MODN_GAME_MODULES_DIR", value);
    }
    if let Some(data) = &backend.data {
        command.env("LOCALAPPDATA", data);
    } else if let Some(value) = std::env::var_os("LOCALAPPDATA") {
        command.env("LOCALAPPDATA", value);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.spawn()
}

/// Whether `value` names an absolute Windows path.
///
/// [`Path::is_absolute`] needs a drive prefix or a UNC share on Windows, so
/// `/tmp` and `\tmp` fail while `C:\tmp` and `\\server\share` pass. Only such
/// values are safe to hand to a Windows child process.
fn is_absolute_windows_path(value: &OsStr) -> bool {
    Path::new(value).is_absolute()
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn only_drive_qualified_temp_paths_are_forwarded() {
        for absolute in [r"C:\Users\J\AppData\Local\Temp", r"\\server\share\tmp"] {
            assert!(
                is_absolute_windows_path(OsStr::new(absolute)),
                "expected an absolute Windows path: {absolute}"
            );
        }
        for relative in ["/tmp", r"\tmp", "tmp", r"C:tmp"] {
            assert!(
                !is_absolute_windows_path(OsStr::new(relative)),
                "must not forward a non-absolute Windows path: {relative}"
            );
        }
    }
}
