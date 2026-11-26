use super::super::registry::layout;
use super::*;

fn write_release(directory: &Path, game: &str, version: &str) -> String {
    fs::create_dir_all(directory).expect("release directory");
    let manifest = serde_json::json!({
        "game_id": game,
        "version": version,
        "api_version": 3,
        "entrypoint": "module.lua",
    })
    .to_string();
    let script = format!("return {{game_id='{game}', api_version=3, version='{version}'}}");
    fs::write(directory.join("module.json"), manifest.as_bytes()).expect("manifest");
    fs::write(directory.join("module.lua"), script.as_bytes()).expect("script");
    let blake3 = layout::digest(manifest.as_bytes(), script.as_bytes());
    let receipt = serde_json::json!({
        "game_id": game,
        "version": version,
        "api_version": 3,
        "blake3": blake3,
        "bucket": "community",
        "signer_key_id": "fixture",
    });
    fs::write(
        directory.join(layout::RECEIPT),
        serde_json::to_vec(&receipt).expect("receipt"),
    )
    .expect("write receipt");
    blake3
}

#[test]
fn a_death_after_the_moves_is_completed_when_the_registry_opens() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let roots = crate::manager_roots::ManagerRoots::new(temp.path(), None);
    let registry = ModuleRegistry::new(&roots);
    let root = registry.root();
    let game = "fixture-game";
    let game_directory = root.join(game);
    fs::create_dir_all(game_directory.join(layout::VERSIONS).join("1.0.0"))
        .expect("old release directory");
    layout::write_current(&game_directory, "1.0.0").expect("old current");
    let staged = root.join(".staging-test").join(game);
    let blake3 = write_release(&staged, game, "2.0.0");
    let mut progress = activation::ActivationProgress::default();
    let mut reports = Vec::new();
    STOP_NEXT_AFTER_MOVES
        .lock()
        .expect("seam")
        .insert(root.clone());

    apply(
        &registry,
        &[StagedRelease {
            game_id: game.to_owned(),
            version: "2.0.0".to_owned(),
            blake3,
            directory: staged,
            newer: true,
            current: Some("1.0.0".to_owned()),
        }],
        &mut progress,
        &mut reports,
    )
    .expect("apply stops after the moves");

    assert!(reports.is_empty(), "the death skips the report");
    assert_eq!(
        fs::read_to_string(game_directory.join(layout::CURRENT)).expect("current"),
        "1.0.0"
    );
    assert!(game_directory.join(layout::VERSIONS).join("2.0.0").is_dir());
    assert!(root.join(activation::ACTIVATION).is_file());

    let opened = ModuleRegistry::open(&roots).expect("open repairs");
    assert!(matches!(
        opened.repair,
        Some(crate::game_modules::ActivationRepair::Completed { .. })
    ));
    assert_eq!(
        opened
            .registry
            .active(game)
            .expect("active release")
            .expect("registered release")
            .module
            .version,
        "2.0.0"
    );
    assert!(!root.join(activation::ACTIVATION).exists());
}

#[test]
fn an_armed_commit_failure_is_scoped_to_one_registry() {
    let here = PathBuf::from("fixture-registry-here");
    let elsewhere = PathBuf::from("fixture-registry-elsewhere");
    super::FAIL_NEXT_CATALOG_COMMIT
        .lock()
        .expect("seam")
        .insert(here.clone());
    let other_failed = std::thread::scope(|scope| {
        scope
            .spawn(|| super::injected_commit_failure(&elsewhere))
            .join()
            .expect("thread")
    });
    assert!(
        !other_failed,
        "arming one registry does not fail another registry's commit"
    );
    assert!(
        super::injected_commit_failure(&here),
        "the armed registry still fails"
    );
    assert!(
        !super::injected_commit_failure(&here),
        "the failure fires once"
    );
}
