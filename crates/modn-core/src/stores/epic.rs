use super::{directory_files, read_bounded, valid_store_id, StoreInstallation, Storefront};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LauncherManifest {
    app_name: String,
    install_location: PathBuf,
    #[serde(default)]
    main_game_app_name: Option<String>,
    #[serde(default, rename = "bIsIncompleteInstall")]
    incomplete_install: bool,
}

/// Games that the Epic Games Launcher manifests in `directory` list.
///
/// Each `.item` file is JSON. Add-on content (whose `MainGameAppName`
/// differs from its `AppName`) and incomplete installations are skipped.
pub(super) fn installations(directory: &Path) -> Vec<StoreInstallation> {
    directory_files(directory)
        .into_iter()
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "item")
        })
        .filter_map(|path| read_bounded(&path))
        .filter_map(|text| serde_json::from_str::<LauncherManifest>(&text).ok())
        .filter(|manifest| {
            !manifest.incomplete_install
                && manifest.install_location.is_absolute()
                && valid_store_id(Storefront::Epic, &manifest.app_name)
                && manifest
                    .main_game_app_name
                    .as_ref()
                    .is_none_or(|main| main == &manifest.app_name)
        })
        .map(|manifest| StoreInstallation {
            store: Storefront::Epic,
            store_id: manifest.app_name,
            path: manifest.install_location,
        })
        .collect()
}
