use super::StoreLibraries;

#[cfg(windows)]
pub(super) fn libraries() -> StoreLibraries {
    use super::GogRecord;
    use std::path::PathBuf;
    use windows_registry::{CURRENT_USER, LOCAL_MACHINE};

    let mut steam_roots = Vec::new();
    let registered_steam = [
        CURRENT_USER
            .open(r"Software\Valve\Steam")
            .and_then(|key| key.get_string("SteamPath")),
        LOCAL_MACHINE
            .open(r"SOFTWARE\WOW6432Node\Valve\Steam")
            .and_then(|key| key.get_string("InstallPath")),
    ];
    for path in registered_steam.into_iter().flatten() {
        steam_roots.push(PathBuf::from(path));
    }
    if let Some(programs) = std::env::var_os("PROGRAMFILES(X86)") {
        steam_roots.push(PathBuf::from(programs).join("Steam"));
    }
    steam_roots.dedup();

    let epic_manifest_dirs = std::env::var_os("PROGRAMDATA")
        .map(|data| {
            vec![PathBuf::from(data)
                .join("Epic")
                .join("EpicGamesLauncher")
                .join("Data")
                .join("Manifests")]
        })
        .unwrap_or_default();

    let mut gog_records = Vec::new();
    for games in [
        r"SOFTWARE\WOW6432Node\GOG.com\Games",
        r"SOFTWARE\GOG.com\Games",
    ] {
        let Ok(games) = LOCAL_MACHINE.open(games) else {
            continue;
        };
        let Ok(names) = games.keys() else {
            continue;
        };
        for name in names {
            let Ok(game) = games.open(&name) else {
                continue;
            };
            if let (Ok(product_id), Ok(path)) = (game.get_string("gameID"), game.get_string("path"))
            {
                gog_records.push(GogRecord {
                    product_id,
                    path: PathBuf::from(path),
                });
            }
        }
    }
    StoreLibraries::new(steam_roots, epic_manifest_dirs, gog_records)
}

#[cfg(not(windows))]
pub(super) fn libraries() -> StoreLibraries {
    StoreLibraries::default()
}
