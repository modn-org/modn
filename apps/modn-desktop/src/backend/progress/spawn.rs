use super::super::Backend;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;
use std::process::{Child, Command, Stdio};

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
