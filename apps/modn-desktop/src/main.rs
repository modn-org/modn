#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod app;
#[cfg(windows)]
#[expect(
    dead_code,
    reason = "The progress-aware write path supersedes the one-shot install and uninstall helpers; the backend contract test still exercises them"
)]
mod backend;

#[cfg(windows)]
fn main() {
    app::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Modn Desktop requires Windows. Use the modn CLI on this platform.");
    std::process::exit(1);
}
