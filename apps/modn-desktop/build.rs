// Build scripts compile for the host, so `cfg(windows)` here means a Windows
// host; the Tauri build dependencies exist only there.
#[cfg(windows)]
const COMMANDS: &[&str] = &[
    "list_games",
    "list_installations",
    "list_packages",
    "catalog",
    "pick_archive",
    "install_archive",
    "uninstall",
    "install_mod",
    "plan_install",
    "update",
    "history",
    "undo",
    "detect_games",
    "activate_game",
    "add_game",
    "remove_game",
    "play",
    "diagnostics",
    "cancel_run",
];

#[cfg(windows)]
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").ok().as_deref() == Some("windows") {
        tauri_build::try_build(
            tauri_build::Attributes::new()
                .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
        )
        .expect("Tauri build must configure the Windows app");
    }
}

#[cfg(not(windows))]
fn main() {}
