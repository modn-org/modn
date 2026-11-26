use modn_core::game_modules::{ActivationRepair, ModuleRegistry, RegistryOpen};
use modn_core::manager_roots::ManagerRoots;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

const GAME_A: &str = "alpha-game";
const GAME_B: &str = "beta-game";
const OLD: &str = "1.0.0";
const NEW: &str = "2.0.0";
const STAGING: &str = ".staging-catalog-repair";

struct Fixture {
    _temp: tempfile::TempDir,
    roots: ManagerRoots,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("temporary directory");
        let roots = ManagerRoots::new(temp.path(), None);
        Self { _temp: temp, roots }
    }

    fn root(&self) -> PathBuf {
        self.roots.module_registry()
    }

    fn release(&self, game: &str, version: &str, marker: &str) -> String {
        write_release(
            &self.root().join(game).join("versions").join(version),
            game,
            version,
            marker,
        )
    }

    fn stage(&self, game: &str, version: &str, marker: &str) -> (PathBuf, String) {
        let relative = Path::new(STAGING).join(game);
        let blake3 = write_release(&self.root().join(&relative), game, version, marker);
        (relative, blake3)
    }

    fn activate(&self, game: &str, version: &str) {
        let directory = self.root().join(game);
        fs::create_dir_all(directory.join("versions")).expect("versions directory");
        fs::write(directory.join("current"), version).expect("current pointer");
    }

    fn current(&self, game: &str) -> String {
        fs::read_to_string(self.root().join(game).join("current"))
            .expect("current pointer")
            .trim_end_matches(['\r', '\n'])
            .to_owned()
    }

    fn write_intent(&self, entries: &[IntentEntry]) {
        let releases = entries
            .iter()
            .map(|entry| {
                json!({
                    "game_id": entry.game_id,
                    "version": entry.version,
                    "blake3": entry.blake3,
                    "previous": entry.previous,
                    "staged": entry.staged.to_string_lossy(),
                })
            })
            .collect::<Vec<_>>();
        let intent = json!({"schema_version": 1, "releases": releases});
        fs::write(
            self.root().join("activation.json"),
            serde_json::to_vec(&intent).expect("activation intent"),
        )
        .expect("write activation intent");
    }

    fn intent_exists(&self) -> bool {
        self.root().join("activation.json").is_file()
    }
}

struct IntentEntry {
    game_id: &'static str,
    version: &'static str,
    blake3: String,
    previous: Option<&'static str>,
    staged: PathBuf,
}

fn write_release(directory: &Path, game: &str, version: &str, marker: &str) -> String {
    fs::create_dir_all(directory).expect("release directory");
    let manifest = json!({
        "game_id": game,
        "version": version,
        "api_version": 3,
        "entrypoint": "module.lua",
    })
    .to_string();
    let script = format!(
        "return {{game_id='{game}', api_version=3, version='{version}', marker='{marker}'}}"
    );
    fs::write(directory.join("module.json"), manifest.as_bytes()).expect("manifest");
    fs::write(directory.join("module.lua"), script.as_bytes()).expect("script");
    let blake3 = digest(manifest.as_bytes(), script.as_bytes());
    let receipt = json!({
        "game_id": game,
        "version": version,
        "api_version": 3,
        "blake3": blake3,
        "bucket": "community",
        "signer_key_id": "fixture",
    });
    fs::write(
        directory.join("registration.json"),
        serde_json::to_vec(&receipt).expect("receipt"),
    )
    .expect("write receipt");
    blake3
}

fn digest(manifest: &[u8], script: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(manifest);
    hasher.update(script);
    hasher.finalize().to_hex().to_string()
}

fn open(fixture: &Fixture) -> RegistryOpen {
    ModuleRegistry::open(&fixture.roots).expect("open registry")
}

fn active(registry: &ModuleRegistry, game: &str) -> String {
    registry
        .active(game)
        .expect("active release")
        .expect("registered release")
        .module
        .version
}

fn unnamed(game: &'static str) -> PathBuf {
    Path::new(STAGING).join(game)
}

#[test]
fn death_after_all_moves_before_any_pointer_completes_the_release() {
    let fixture = Fixture::new();
    fixture.activate(GAME_A, OLD);
    let blake3 = fixture.release(GAME_A, NEW, "new");
    fixture.write_intent(&[IntentEntry {
        game_id: GAME_A,
        version: NEW,
        blake3,
        previous: Some(OLD),
        staged: unnamed(GAME_A),
    }]);

    let opened = open(&fixture);

    assert!(matches!(
        opened.repair,
        Some(ActivationRepair::Completed { .. })
    ));
    assert_eq!(fixture.current(GAME_A), NEW);
    assert_eq!(active(&opened.registry, GAME_A), NEW);
    assert!(!fixture.intent_exists(), "the intent is removed");
}

#[test]
fn death_after_some_pointers_completes_every_module_in_the_set() {
    let fixture = Fixture::new();
    fixture.activate(GAME_A, OLD);
    fixture.activate(GAME_B, OLD);
    let alpha = fixture.release(GAME_A, NEW, "alpha");
    let beta = fixture.release(GAME_B, NEW, "beta");
    fixture.activate(GAME_A, NEW);
    fixture.write_intent(&[
        IntentEntry {
            game_id: GAME_A,
            version: NEW,
            blake3: alpha,
            previous: Some(OLD),
            staged: unnamed(GAME_A),
        },
        IntentEntry {
            game_id: GAME_B,
            version: NEW,
            blake3: beta,
            previous: Some(OLD),
            staged: unnamed(GAME_B),
        },
    ]);

    let opened = open(&fixture);

    assert!(matches!(
        opened.repair,
        Some(ActivationRepair::Completed { .. })
    ));
    assert_eq!(fixture.current(GAME_A), NEW);
    assert_eq!(fixture.current(GAME_B), NEW);
    assert_eq!(active(&opened.registry, GAME_A), NEW);
    assert_eq!(active(&opened.registry, GAME_B), NEW);
    assert!(!fixture.intent_exists(), "the intent is removed");
}

#[test]
fn a_tampered_release_is_not_activated_and_the_set_rolls_back() {
    let fixture = Fixture::new();
    fixture.activate(GAME_A, OLD);
    fixture.activate(GAME_B, OLD);
    fixture.release(GAME_A, OLD, "old-alpha");
    fixture.release(GAME_B, OLD, "old-beta");
    let alpha = fixture.release(GAME_A, NEW, "alpha");
    let beta = fixture.release(GAME_B, NEW, "beta");
    let tampered = fixture
        .root()
        .join(GAME_B)
        .join("versions")
        .join(NEW)
        .join("module.lua");
    fs::write(
        &tampered,
        b"return {game_id='beta-game', api_version=3, version='2.0.0', marker='tampered'}",
    )
    .expect("tamper beta script");
    let tampered_bytes = fs::read(&tampered).expect("tampered beta script");
    fixture.write_intent(&[
        IntentEntry {
            game_id: GAME_A,
            version: NEW,
            blake3: alpha,
            previous: Some(OLD),
            staged: unnamed(GAME_A),
        },
        IntentEntry {
            game_id: GAME_B,
            version: NEW,
            blake3: beta,
            previous: Some(OLD),
            staged: unnamed(GAME_B),
        },
    ]);

    let opened = open(&fixture);

    match opened.repair {
        Some(ActivationRepair::RolledBack { reason, .. }) => {
            assert!(reason.contains("do not match"), "{reason}");
        }
        other => panic!("expected a rollback, got {other:?}"),
    }
    assert_eq!(fixture.current(GAME_A), OLD);
    assert_eq!(fixture.current(GAME_B), OLD);
    assert_eq!(
        fs::read(&tampered).expect("beta script after repair"),
        tampered_bytes,
        "repair never writes or replaces module bytes"
    );
    assert!(
        fixture
            .root()
            .join(GAME_B)
            .join("versions")
            .join(NEW)
            .is_dir(),
        "the tampered directory is not activated or removed"
    );
    assert_eq!(active(&opened.registry, GAME_A), OLD);
    assert_eq!(active(&opened.registry, GAME_B), OLD);
    assert!(!fixture.intent_exists(), "the intent is removed");
}

#[test]
fn death_in_the_middle_of_the_moves_finishes_the_staged_release() {
    let fixture = Fixture::new();
    fixture.activate(GAME_A, OLD);
    fixture.activate(GAME_B, OLD);
    let alpha = fixture.release(GAME_A, NEW, "alpha");
    let (staged_beta, beta) = fixture.stage(GAME_B, NEW, "beta");
    fixture.write_intent(&[
        IntentEntry {
            game_id: GAME_A,
            version: NEW,
            blake3: alpha,
            previous: Some(OLD),
            staged: unnamed(GAME_A),
        },
        IntentEntry {
            game_id: GAME_B,
            version: NEW,
            blake3: beta,
            previous: Some(OLD),
            staged: staged_beta.clone(),
        },
    ]);

    let opened = open(&fixture);

    assert!(matches!(
        opened.repair,
        Some(ActivationRepair::Completed { .. })
    ));
    assert_eq!(fixture.current(GAME_A), NEW);
    assert_eq!(fixture.current(GAME_B), NEW);
    assert_eq!(active(&opened.registry, GAME_A), NEW);
    assert_eq!(active(&opened.registry, GAME_B), NEW);
    assert!(
        !fixture.root().join(&staged_beta).exists(),
        "the staged directory was moved into the registry"
    );
    assert!(!fixture.intent_exists(), "the intent is removed");
}

#[test]
fn a_corrupt_intent_is_reported_and_the_registry_stays_readable() {
    let fixture = Fixture::new();
    fixture.activate(GAME_A, OLD);
    fixture.release(GAME_A, OLD, "old");
    fs::write(fixture.root().join("activation.json"), b"{ not json")
        .expect("corrupt activation intent");

    let opened = open(&fixture);

    assert!(matches!(
        opened.repair,
        Some(ActivationRepair::Unreadable { .. })
    ));
    assert_eq!(active(&opened.registry, GAME_A), OLD);
    assert_eq!(
        opened.registry.list().expect("readable registry").len(),
        1,
        "the corrupt intent is not treated as a game directory"
    );
    assert!(
        fixture.intent_exists(),
        "the unreadable intent is left in place"
    );
}
