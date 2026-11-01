//! Uninstall by catalog name, with dependencies and dependents.
use super::catalog_tests::{module_source, reference, Fixture};
use super::{CatalogDependency, CatalogReference, CatalogRelease, UninstallScope};
use crate::archive::inspect;
use crate::game_modules::with_module_source;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

const DEPENDENCIES: UninstallScope = UninstallScope {
    dependencies: true,
    dependents: false,
};
const DEPENDENTS: UninstallScope = UninstallScope {
    dependencies: false,
    dependents: true,
};
const NEITHER: UninstallScope = UninstallScope {
    dependencies: false,
    dependents: false,
};

/// A game fixture whose mods each have one archive, built once so that
/// dependency digests match the archive that was installed.
struct Lab {
    fixture: Fixture,
    archives: HashMap<String, PathBuf>,
}

impl Lab {
    fn new() -> Self {
        Self {
            fixture: Fixture::new(),
            archives: HashMap::new(),
        }
    }

    /// Install `name` from `bucket`, needing the already-installed mods in `needs`.
    fn install_from(&mut self, bucket: &str, name: &str, needs: &[&str]) -> String {
        let dependencies = needs
            .iter()
            .map(|need| CatalogDependency {
                reference: reference(need),
                archive_sha256: inspect(&self.archives[*need]).unwrap().sha256,
            })
            .collect();
        let archive = self
            .archives
            .entry(name.to_owned())
            .or_insert_with(|| self.fixture.archive(name))
            .clone();
        let mut mod_reference: CatalogReference = reference(name);
        mod_reference.bucket = bucket.to_owned();
        let release = CatalogRelease::new(mod_reference, dependencies);
        self.fixture.install_release(&archive, release).unwrap().id
    }

    fn install(&mut self, name: &str, needs: &[&str]) -> String {
        self.install_from("deps", name, needs)
    }

    fn uninstall(&mut self, selector: &str, scope: UninstallScope) -> anyhow::Result<Vec<String>> {
        let game = self.fixture.game.clone();
        with_module_source(Some(&module_source()), || {
            self.fixture.store.uninstall_with(&game, selector, scope)
        })
    }

    /// Mod names still installed, sorted: packages made in one second have no stable order.
    fn installed(&self) -> Vec<String> {
        let packages = self.fixture.store.installed(&self.fixture.game).unwrap();
        let mut names: Vec<String> = packages
            .into_iter()
            .map(|p| p.catalog.unwrap().mod_id)
            .collect();
        names.sort();
        names
    }

    fn plan(&self, selector: &str, scope: UninstallScope) -> anyhow::Result<Vec<String>> {
        let plan = self
            .fixture
            .store
            .plan_uninstall(&self.fixture.game, selector, scope)?;
        Ok(plan.into_iter().map(|p| p.id).collect())
    }
}

#[test]
fn a_package_is_selected_by_id_or_by_any_form_of_its_catalog_name() {
    let mut lab = Lab::new();
    let id = lab.install("solo", &[]);
    for selector in [id.as_str(), "solo", "test-game/solo", "deps/test-game/solo"] {
        assert_eq!(
            lab.plan(selector, NEITHER).unwrap(),
            std::slice::from_ref(&id),
            "{selector}"
        );
    }
    let missing = lab.plan("other", NEITHER).unwrap_err().to_string();
    assert!(
        missing.contains("no managed package matches other"),
        "{missing}"
    );

    assert_eq!(lab.uninstall("solo", NEITHER).unwrap(), [id]);
    assert!(lab.installed().is_empty());
    assert!(!lab.fixture.game.join("addons/solo.plugin").exists());
}

#[test]
fn a_name_that_matches_several_packages_is_refused() {
    let mut lab = Lab::new();
    let first = lab.install_from("deps", "twin", &[]);
    let second = lab.install_from("alt", "twin-two", &[]);
    // Give the second package the same mod name in another bucket.
    lab.fixture
        .store
        .connection
        .execute(
            "UPDATE catalog_provenance SET mod_id='twin' WHERE package_id=?1",
            [&second],
        )
        .unwrap();

    let error = lab.plan("twin", NEITHER).unwrap_err().to_string();
    assert!(error.contains("matches several"), "{error}");
    assert!(error.contains(&first) && error.contains(&second), "{error}");
    assert_eq!(lab.plan("deps/test-game/twin", NEITHER).unwrap(), [first]);
    assert_eq!(lab.plan("alt/test-game/twin", NEITHER).unwrap(), [second]);
}

#[test]
fn dependents_block_the_selection_unless_they_are_included() {
    let mut lab = Lab::new();
    let base = lab.install("base", &[]);
    let middle = lab.install("middle", &["base"]);
    let top = lab.install("top", &["middle"]);

    let message = lab.uninstall("base", NEITHER).unwrap_err().to_string();
    assert!(message.contains("is required by"), "{message}");
    assert!(message.contains("deps/test-game/middle"), "{message}");
    assert_eq!(lab.installed(), ["base", "middle", "top"]);

    let order = [top, middle, base];
    assert_eq!(lab.plan("base", DEPENDENTS).unwrap(), order);
    assert_eq!(lab.installed().len(), 3, "planning writes nothing");
    assert_eq!(lab.uninstall("base", DEPENDENTS).unwrap(), order);
    assert!(lab.installed().is_empty());
}

#[test]
fn dependencies_leave_only_when_no_remaining_package_needs_them() {
    let mut lab = Lab::new();
    let base = lab.install("base", &[]);
    let first = lab.install("first", &["base"]);
    let second = lab.install("second", &["base"]);

    // `second` still needs `base`, so only `first` goes.
    assert_eq!(lab.uninstall("first", DEPENDENCIES).unwrap(), [first]);
    assert_eq!(lab.installed(), ["base", "second"]);
    assert_eq!(
        lab.uninstall("second", DEPENDENCIES).unwrap(),
        [second, base]
    );
    assert!(lab.installed().is_empty());
}

#[test]
fn without_the_scope_dependencies_stay_installed() {
    let mut lab = Lab::new();
    lab.install("base", &[]);
    lab.install("top", &["base"]);
    lab.uninstall("top", NEITHER).unwrap();
    assert_eq!(lab.installed(), ["base"]);
}

#[test]
fn a_failed_removal_names_what_was_removed_and_keeps_the_rest() {
    let mut lab = Lab::new();
    let base = lab.install("base", &[]);
    let top = lab.install("top", &["base"]);
    let edited = lab.fixture.game.join("addons/base.plugin");
    fs::write(&edited, b"edited").unwrap();

    let error = lab.uninstall("top", DEPENDENCIES).unwrap_err();
    let message = format!("{error:#}");
    assert!(
        message.contains(&format!("stopped at {base} after removing [{top}]")),
        "{message}"
    );
    assert!(
        message.contains("owned file missing or modified"),
        "{message}"
    );
    assert_eq!(lab.installed(), ["base"]);
    assert_eq!(fs::read(&edited).unwrap(), b"edited");
}
