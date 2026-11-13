use super::{GogRecord, StoreInstallation, StoreLibraries, Storefront};
use std::fs;
use std::path::{Path, PathBuf};

fn vdf_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "\\\\")
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("fixture folder");
    fs::write(path, text).expect("fixture file");
}

fn app_manifest(library: &Path, app_id: &str, folder: &str) {
    write(
        &library.join(format!("steamapps/appmanifest_{app_id}.acf")),
        &format!(
            "\"AppState\"\n{{\n\t\"appid\"\t\t\"{app_id}\"\n\t\"name\"\t\t\"Fixture\"\n\t\"installdir\"\t\t\"{folder}\"\n}}\n"
        ),
    );
}

fn installation(store: Storefront, store_id: &str, path: PathBuf) -> StoreInstallation {
    StoreInstallation {
        store,
        store_id: store_id.into(),
        path,
    }
}

#[test]
fn steam_reads_the_root_and_every_listed_library() {
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path().join("Steam");
    let extra = temp.path().join("SteamLibrary");
    write(
        &root.join("steamapps/libraryfolders.vdf"),
        &format!(
            "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t\t\"apps\"\n\t\t{{\n\t\t\t\"1091500\"\t\t\"1\"\n\t\t}}\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
            vdf_path(&root),
            vdf_path(&extra)
        ),
    );
    app_manifest(&root, "1091500", "Cyberpunk 2077");
    app_manifest(&extra, "2379780", "Balatro");

    let libraries = StoreLibraries::new(vec![root.clone()], Vec::new(), Vec::new());

    assert_eq!(
        libraries.installations(),
        vec![
            installation(
                Storefront::Steam,
                "1091500",
                root.join("steamapps/common/Cyberpunk 2077")
            ),
            installation(
                Storefront::Steam,
                "2379780",
                extra.join("steamapps/common/Balatro")
            ),
        ]
    );
}

#[test]
fn steam_skips_unsafe_and_mismatched_app_manifests() {
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path().join("Steam");
    app_manifest(&root, "10", "..");
    app_manifest(&root, "11", "nested\\\\folder");
    write(
        &root.join("steamapps/appmanifest_12.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\t\"13\"\n\t\"installdir\"\t\t\"Game\"\n}\n",
    );
    write(&root.join("steamapps/appmanifest_x.acf"), "");

    let libraries = StoreLibraries::new(vec![root], Vec::new(), Vec::new());

    assert_eq!(libraries.installations(), Vec::new());
}

#[test]
fn epic_reads_complete_base_game_manifests() {
    let temp = tempfile::tempdir().expect("temp");
    let manifests = temp.path().join("Manifests");
    let game = temp.path().join("Games/Cyberpunk2077");
    let entry = |app: &str, main: &str, incomplete: bool| {
        serde_json::json!({
            "FormatVersion": 0,
            "bIsIncompleteInstall": incomplete,
            "DisplayName": "Fixture",
            "InstallLocation": game,
            "AppName": app,
            "MainGameAppName": main,
        })
        .to_string()
    };
    write(&manifests.join("A.item"), &entry("Ginger", "Ginger", false));
    write(
        &manifests.join("B.item"),
        &entry("GingerDlc", "Ginger", false),
    );
    write(&manifests.join("C.item"), &entry("Other", "Other", true));
    write(&manifests.join("D.txt"), &entry("Text", "Text", false));
    write(&manifests.join("E.item"), "not json");

    let libraries = StoreLibraries::new(Vec::new(), vec![manifests], Vec::new());

    assert_eq!(
        libraries.installations(),
        vec![installation(Storefront::Epic, "Ginger", game)]
    );
}

#[test]
fn gog_reads_registry_records_with_numeric_ids() {
    let temp = tempfile::tempdir().expect("temp");
    let game = temp.path().join("GOG Games/Cyberpunk 2077");
    let record = |product_id: &str, path: PathBuf| GogRecord {
        product_id: product_id.into(),
        path,
    };
    let libraries = StoreLibraries::new(
        Vec::new(),
        Vec::new(),
        vec![
            record("1423049311", game.clone()),
            record("not-a-number", game.clone()),
            record("1", PathBuf::from("relative")),
        ],
    );

    assert_eq!(
        libraries.installations(),
        vec![installation(Storefront::Gog, "1423049311", game)]
    );
}

#[test]
fn test_builds_read_only_the_fixture_named_by_the_environment() {
    assert!(std::env::var_os("MODN_STORE_LIBRARIES").is_none());
    assert_eq!(StoreLibraries::system(), StoreLibraries::default());
}
