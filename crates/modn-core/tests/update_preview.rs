#[path = "support/main_bucket.rs"]
mod main_bucket;
use modn_core::planning::{
    preview_install_zip_with_permissions, preview_update_with_permissions, PlanStatus,
};
use modn_core::state::Store;
use modn_core::transactions::{CatalogReference, CatalogRelease};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;

const GAME: &str = "cyberpunk-2077";

fn plugin(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    let mut zip = zip::ZipWriter::new(File::create(&path).expect("archive"));
    zip.start_file("red4ext/plugins/Foo/Foo.dll", SimpleFileOptions::default())
        .expect("entry");
    zip.write_all(contents.as_bytes()).expect("payload");
    zip.finish().expect("finish");
    path
}

#[test]
fn update_preview_plans_as_if_the_installed_release_were_removed() {
    main_bucket::seeded(|| {
        let temp = tempfile::tempdir().expect("temp");
        let game = temp.path().join("game");
        fs::create_dir_all(game.join("bin/x64")).expect("game");
        fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").expect("marker");
        fs::create_dir_all(game.join("red4ext/plugins")).expect("plugins");
        let old = plugin(temp.path(), "foo-1.zip", "v1");
        let new = plugin(temp.path(), "foo-2.zip", "v2");
        let mut store = Store::open(&temp.path().join("state/state.sqlite")).expect("state");
        let release = CatalogRelease::new(
            CatalogReference::new("main".into(), GAME.into(), "foo".into()),
            Vec::new(),
        );
        let first = preview_install_zip_with_permissions(GAME, &game, &old, &[]).expect("preview");
        let installed = store
            .install_catalog_release(&game, &old, &first, &release)
            .expect("install");
        assert_eq!(installed.directories, ["red4ext/plugins/Foo"]);
        fs::write(game.join("red4ext/plugins/Foo/runtime.log"), b"log").expect("log");

        let naive = preview_install_zip_with_permissions(GAME, &game, &new, &[]).expect("naive");
        assert_eq!(naive.status, PlanStatus::Blocked);
        assert_eq!(naive.operations[0].owned_directory, None);

        let mut replaced: Vec<String> = installed
            .files
            .iter()
            .map(|file| file.destination.clone())
            .collect();
        replaced.extend(installed.directories.iter().cloned());
        let plan = preview_update_with_permissions(GAME, &game, &new, &[], &replaced)
            .expect("update plan");
        assert_eq!(plan.status, PlanStatus::Reviewable, "{:?}", plan.blockers);
        assert_eq!(
            plan.operations[0].owned_directory.as_deref(),
            Some("red4ext/plugins/Foo")
        );

        let updated = store
            .update_catalog_release(&game, &installed.id, &new, &plan, &release, &[])
            .expect("update");
        assert_eq!(updated.directories, ["red4ext/plugins/Foo"]);
        assert_eq!(
            fs::read_to_string(game.join("red4ext/plugins/Foo/Foo.dll")).expect("plugin"),
            "v2"
        );
        assert!(!game.join("red4ext/plugins/Foo/runtime.log").exists());
        assert_eq!(store.installed(&game).expect("installed").len(), 1);
        assert_eq!(store.recover().expect("recover"), 0);
    });
}
