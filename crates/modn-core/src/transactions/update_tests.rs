use super::catalog_tests::{plan_into, reference, Fixture};
use super::install::Fault;
use super::journal::{target_path, temp_path};
use super::{CatalogDependency, CatalogRelease, ManagedPackage, NativeInjectorPolicy};
use crate::archive::inspect;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;

fn release_archive(dir: &Path, name: &str, files: &[(&str, &str)]) -> PathBuf {
    let path = dir.join(format!("{name}.zip"));
    let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
    for (file, contents) in files {
        zip.start_file(format!("pack/{file}"), SimpleFileOptions::default())
            .unwrap();
        zip.write_all(contents.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    path
}

struct Installed {
    fixture: Fixture,
    archives: TempDir,
    base: ManagedPackage,
    top: ManagedPackage,
    new_base: PathBuf,
}

fn base_with_dependent() -> Installed {
    let mut fixture = Fixture::new();
    let archives = tempfile::tempdir().unwrap();
    let old_base = release_archive(
        archives.path(),
        "base-1",
        &[("base.plugin", "v1"), ("retired.plugin", "old only")],
    );
    let new_base = release_archive(
        archives.path(),
        "base-2",
        &[("base.plugin", "v2"), ("added.plugin", "new only")],
    );
    let top = release_archive(archives.path(), "top", &[("top.plugin", "top")]);
    let base = fixture
        .install_release(
            &old_base,
            CatalogRelease::new(reference("base"), Vec::new()),
        )
        .unwrap();
    let needs_base = CatalogDependency::new(reference("base"), inspect(&old_base).unwrap().sha256);
    let top = fixture
        .install_release(
            &top,
            CatalogRelease::new(reference("top"), vec![needs_base]),
        )
        .unwrap();
    Installed {
        fixture,
        archives,
        base,
        top,
        new_base,
    }
}

#[test]
fn pin_after_preview_blocks_update_before_withdrawal() {
    let mut installed = base_with_dependent();
    let game = installed.fixture.game.clone();
    let proposal = plan_into(&game, &installed.new_base, "addons");
    installed
        .fixture
        .store
        .connection
        .execute(
            "UPDATE package_requirements SET requirement='<2.0' WHERE package_id=?1",
            [&installed.top.id],
        )
        .expect("requirement");
    installed
        .fixture
        .store
        .set_package_pin(&game, &installed.top.id, true)
        .expect("pin after preview");
    let before = fs::read(game.join("addons/base.plugin")).expect("old content");
    let error = installed
        .fixture
        .store
        .update_catalog_release(
            &game,
            &installed.base.id,
            &installed.new_base,
            &proposal,
            &CatalogRelease::new(reference("base"), vec![]).with_version("2.0".into()),
            NativeInjectorPolicy::Refuse,
            &[],
        )
        .expect_err("pinned dependent blocks update");
    assert!(error.to_string().contains("pinned"));
    assert!(error.to_string().contains(&installed.top.id));
    assert_eq!(
        fs::read(game.join("addons/base.plugin")).expect("content"),
        before
    );
    installed.fixture.store.ensure_idle().expect("no journal");
    assert!(installed
        .fixture
        .store
        .installed(&game)
        .expect("packages")
        .iter()
        .any(|p| p.id == installed.base.id));
}

impl Installed {
    fn update(&mut self, fault: Option<Fault>) -> anyhow::Result<ManagedPackage> {
        self.update_to(CatalogRelease::new(reference("base"), Vec::new()), fault)
    }

    fn update_to(
        &mut self,
        release: CatalogRelease,
        fault: Option<Fault>,
    ) -> anyhow::Result<ManagedPackage> {
        let game = self.fixture.game.clone();
        let proposal = plan_into(&game, &self.new_base, "addons");
        self.fixture.store.update_inner(
            &game,
            &self.base.id,
            &self.new_base,
            &proposal,
            &release,
            &[],
            NativeInjectorPolicy::Refuse,
            fault,
        )
    }

    fn read(&self, file: &str) -> Option<String> {
        fs::read_to_string(self.fixture.game.join("addons").join(file)).ok()
    }

    fn package_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .fixture
            .store
            .installed(&self.fixture.game)
            .unwrap()
            .into_iter()
            .map(|package| package.id)
            .collect();
        ids.sort();
        ids
    }

    fn dependents(&self, package_id: &str) -> Vec<String> {
        self.fixture
            .store
            .dependents_of(&self.fixture.game, package_id)
            .unwrap()
            .into_iter()
            .map(|package| package.id)
            .collect()
    }

    fn assert_old_release_intact(&self, ids_before: &[String]) {
        assert_eq!(self.read("base.plugin").as_deref(), Some("v1"));
        assert_eq!(self.read("retired.plugin").as_deref(), Some("old only"));
        assert_eq!(self.read("added.plugin"), None);
        assert_eq!(self.package_ids(), ids_before);
        assert_eq!(
            self.dependents(&self.base.id),
            std::slice::from_ref(&self.top.id)
        );
        assert!(self.fixture.store.pending().unwrap().is_empty());
        assert_no_transaction_files(&self.fixture.game.join("addons"));
    }
}

#[test]
fn target_pinned_after_planning_stays_installed() {
    let mut installed = base_with_dependent();
    let game = installed.fixture.game.clone();
    installed
        .fixture
        .store
        .set_package_pin(&game, &installed.base.id, true)
        .expect("pin target");
    let error = installed.update(None).expect_err("pinned target");
    assert!(error.to_string().contains("pinned"));
    assert_eq!(
        fs::read_to_string(game.join("addons/base.plugin")).expect("old content"),
        "v1"
    );
    installed.fixture.store.ensure_idle().expect("no journal");
}

fn assert_no_transaction_files(folder: &Path) {
    for entry in fs::read_dir(folder).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        assert!(!name.starts_with(".modn-"), "leftover {name}");
    }
}

#[test]
fn update_replaces_the_release_and_moves_its_dependents() {
    let mut installed = base_with_dependent();
    let updated = installed.update(None).unwrap();

    assert_eq!(installed.read("base.plugin").as_deref(), Some("v2"));
    assert_eq!(installed.read("added.plugin").as_deref(), Some("new only"));
    assert_eq!(installed.read("retired.plugin"), None);
    assert_ne!(updated.id, installed.base.id);
    assert_eq!(updated.catalog, Some(reference("base")));
    let mut expected = vec![updated.id.clone(), installed.top.id.clone()];
    expected.sort();
    assert_eq!(installed.package_ids(), expected);
    assert_eq!(
        installed.dependents(&updated.id),
        [installed.top.id.clone()]
    );
    assert!(installed.dependents(&installed.base.id).is_empty());
    assert!(installed.fixture.store.pending().unwrap().is_empty());
    assert_no_transaction_files(&installed.fixture.game.join("addons"));
    let blocked = installed.fixture.uninstall(&updated.id).unwrap_err();
    assert!(blocked.to_string().contains("required by"), "{blocked}");
}

#[test]
fn failed_update_restores_the_old_release() {
    let mut installed = base_with_dependent();
    let before = installed.package_ids();
    let error = installed.update(Some(Fault::FailAfter(2))).unwrap_err();
    assert!(format!("{error:#}").contains("rolled back"), "{error:#}");
    installed.assert_old_release_intact(&before);
    assert_eq!(installed.fixture.store.recover().unwrap(), 0);
    installed.update(None).unwrap();
    assert_eq!(installed.read("base.plugin").as_deref(), Some("v2"));
}

#[test]
fn recovery_restores_the_old_release_after_an_interrupted_update() {
    let mut installed = base_with_dependent();
    let before = installed.package_ids();
    installed.update(Some(Fault::CrashAfter(2))).unwrap_err();
    assert_eq!(installed.fixture.store.pending().unwrap().len(), 2);
    assert_eq!(installed.read("base.plugin").as_deref(), Some("v2"));
    assert_eq!(installed.read("retired.plugin"), None);
    let blocked = installed.update(None).unwrap_err();
    assert!(
        blocked.to_string().contains("unfinished transaction"),
        "{blocked}"
    );

    assert_eq!(installed.fixture.store.recover().unwrap(), 2);
    installed.assert_old_release_intact(&before);
}

#[test]
fn recovery_restores_the_old_release_after_an_interrupted_copy() {
    let mut installed = base_with_dependent();
    let before = installed.package_ids();
    installed.update(Some(Fault::CrashAfter(1))).unwrap_err();
    let pending = installed.fixture.store.pending().unwrap();
    let copy = pending
        .iter()
        .find(|journal| journal.action == "install")
        .unwrap();
    let game = installed.fixture.game.clone();
    let unlinked = &copy.files[1];
    let destination = target_path(&game, &unlinked.destination).unwrap();
    assert!(!destination.exists());
    let partial = temp_path(&destination, &copy.id, unlinked.ordinal).unwrap();
    fs::write(&partial, "v").unwrap();

    assert_eq!(installed.fixture.store.recover().unwrap(), 2);
    assert!(!partial.exists());
    installed.assert_old_release_intact(&before);
}

#[test]
fn recovery_keeps_a_linked_witness_that_another_process_changed() {
    let mut installed = base_with_dependent();
    installed.update(Some(Fault::CrashAfter(1))).unwrap_err();
    let pending = installed.fixture.store.pending().unwrap();
    let copy = pending
        .iter()
        .find(|journal| journal.action == "install")
        .unwrap();
    let linked = &copy.files[0];
    let game = installed.fixture.game.clone();
    let destination = target_path(&game, &linked.destination).unwrap();
    let witness = temp_path(&destination, &copy.id, linked.ordinal).unwrap();
    fs::write(&destination, "user data").unwrap();
    fs::remove_file(&destination).unwrap();

    let error = installed.fixture.store.recover().unwrap_err();
    assert!(
        error.to_string().contains("changed or incomplete witness"),
        "{error:#}"
    );
    assert_eq!(fs::read_to_string(&witness).unwrap(), "user data");
}

#[test]
fn update_refuses_a_release_that_depends_on_its_own_dependent() {
    let mut installed = base_with_dependent();
    let before = installed.package_ids();
    let needs_top = CatalogDependency::new(reference("top"), installed.top.archive_sha256.clone());
    let error = installed
        .update_to(
            CatalogRelease::new(reference("base"), vec![needs_top]),
            None,
        )
        .unwrap_err();
    assert!(error.to_string().contains("dependency cycle"), "{error:#}");
    installed.assert_old_release_intact(&before);
}

#[test]
fn update_never_overwrites_an_unmanaged_file() {
    let mut installed = base_with_dependent();
    let before = installed.package_ids();
    let unmanaged = installed.fixture.game.join("addons/added.plugin");
    fs::write(&unmanaged, "personal").unwrap();

    assert!(installed.update(None).is_err());
    assert_eq!(fs::read_to_string(&unmanaged).unwrap(), "personal");
    fs::remove_file(&unmanaged).unwrap();
    installed.assert_old_release_intact(&before);
    drop(installed.archives);
}
