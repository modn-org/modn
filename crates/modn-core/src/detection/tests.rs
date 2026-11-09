use super::detect_installations;
use crate::game_modules::with_module_source;
use crate::stores::{GogRecord, StoreLibraries, Storefront};
use std::fs;
use std::path::{Path, PathBuf};

const MODULE_JSON: &str = r#"{"game_id":"store-game","version":"0.1.0","api_version":1,"entrypoint":"main.lua","stores":{"steam":"100","gog":"200","epic":"StoreGame"}}"#;
const MODULE_LUA: &str = r#"
local policy = { game_id = "store-game", api_version = mods_host.api_version, marker = "Game.exe" }
function policy.validate()
    local ok, item = pcall(mods_host.fs.metadata, 1, "Game.exe")
    return ok and item.kind == "file"
end
return policy
"#;

fn module(root: &Path) -> PathBuf {
    let module = root.join("module");
    fs::create_dir_all(&module).expect("module folder");
    fs::write(module.join("module.json"), MODULE_JSON).expect("manifest");
    fs::write(module.join("main.lua"), MODULE_LUA).expect("script");
    module
}

fn game(path: &Path) -> PathBuf {
    fs::create_dir_all(path).expect("game folder");
    fs::write(path.join("Game.exe"), b"fixture").expect("marker");
    path.canonicalize().expect("canonical game")
}

fn epic_manifest(directory: &Path, location: &Path) {
    fs::create_dir_all(directory).expect("manifest folder");
    let entry = serde_json::json!({"AppName": "StoreGame", "InstallLocation": location});
    fs::write(directory.join("game.item"), entry.to_string()).expect("manifest");
}

#[test]
fn confirms_each_store_folder_and_refuses_unrecognized_folders() {
    let temp = tempfile::tempdir().expect("temp");
    let module = module(temp.path());
    let gog = game(&temp.path().join("gog/Store Game"));
    let epic = game(&temp.path().join("epic/StoreGame"));
    let wrong = temp.path().join("other/Wrong");
    fs::create_dir_all(&wrong).expect("wrong folder");
    let manifests = temp.path().join("manifests");
    epic_manifest(&manifests, &epic);
    let libraries = StoreLibraries::new(
        Vec::new(),
        vec![manifests],
        vec![
            GogRecord {
                product_id: "200".into(),
                path: gog.clone(),
            },
            GogRecord {
                product_id: "200".into(),
                path: wrong.clone(),
            },
            GogRecord {
                product_id: "999".into(),
                path: gog.clone(),
            },
        ],
    );

    let detection = with_module_source(Some(&module), || {
        detect_installations(&libraries, &["store-game".to_owned()])
    });

    let confirmed: Vec<_> = detection
        .confirmed
        .iter()
        .map(|found| (found.store, found.store_id.as_str(), found.path.clone()))
        .collect();
    let mut expected = vec![
        (Storefront::Epic, "StoreGame", epic),
        (Storefront::Gog, "200", gog),
    ];
    expected.sort_by(|left, right| left.2.cmp(&right.2));
    assert_eq!(confirmed, expected);
    assert_eq!(detection.refused.len(), 1);
    assert_eq!(detection.refused[0].path, wrong);
    assert!(detection.refused[0]
        .reason
        .starts_with("DETECT_FOLDER_UNCONFIRMED:"));
    assert!(detection.refused[0].reason.contains("Game.exe"));
    assert!(detection.skipped.is_empty());
}

#[test]
fn reports_one_folder_once_and_skips_unreadable_modules() {
    let temp = tempfile::tempdir().expect("temp");
    let module = module(temp.path());
    let folder = game(&temp.path().join("Store Game"));
    let record = GogRecord {
        product_id: "200".into(),
        path: folder.clone(),
    };
    let libraries = StoreLibraries::new(Vec::new(), Vec::new(), vec![record.clone(), record]);

    let detection = with_module_source(Some(&module), || {
        detect_installations(
            &libraries,
            &["store-game".to_owned(), "other-game".to_owned()],
        )
    });

    assert_eq!(detection.confirmed.len(), 1);
    assert_eq!(detection.confirmed[0].path, folder);
    assert_eq!(detection.skipped.len(), 1);
    assert_eq!(detection.skipped[0].game_id, "other-game");
    assert!(detection.skipped[0]
        .reason
        .starts_with("DETECT_MODULE_UNREADABLE:"));
}
