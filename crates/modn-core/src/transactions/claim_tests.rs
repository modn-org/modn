use super::claim::PlanOverride;
use crate::planning::{existing_destination_issue, InstallPlan, PlanStatus, PlannedFile};
use crate::state::Store;
use rusqlite::params;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};

const SHARED: &str = "archive/pc/mod/shared.archive";

struct Fixture {
    _root: TempDir,
    store: Store,
    game: PathBuf,
    profile: String,
}

impl Fixture {
    fn new() -> Self {
        let root = tempdir().expect("fixture");
        let game = root.path().join("game");
        fs::create_dir_all(game.join("bin/x64")).expect("bin");
        fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").expect("exe");
        let game = game.canonicalize().expect("canonical game");
        let store = Store::open(&root.path().join("state/state.sqlite")).expect("state");
        let profile = crate::state::profiles::ensure_profile(&store.connection, &game)
            .expect("default profile")
            .id;
        Self {
            _root: root,
            store,
            game,
            profile,
        }
    }

    fn package(&self, id: &str, priority: Option<i64>) {
        self.store
            .connection
            .execute(
                "INSERT INTO managed_packages(id,game_path,archive_blake3) VALUES (?1,?2,'hash')",
                params![id, self.game.to_string_lossy().as_ref()],
            )
            .expect("package");
        if let Some(priority) = priority {
            self.store
                .connection
                .execute(
                    "INSERT INTO profile_packages(profile_id,package_id,priority,enabled)
                     VALUES (?1,?2,?3,1)",
                    params![self.profile, id, priority],
                )
                .expect("membership");
        }
    }

    fn file(&self, package: &str, destination: &str, blake3: &str, bytes: u64) {
        self.store
            .connection
            .execute(
                "INSERT INTO managed_files
                 (game_path,root,path_key,relative_path,package_id,blake3,bytes,archive_entry)
                 VALUES (?1,'game',?2,?3,?4,?5,?6,?3)",
                params![
                    self.game.to_string_lossy().as_ref(),
                    super::ownership_key(destination),
                    destination,
                    package,
                    blake3,
                    bytes as i64
                ],
            )
            .expect("managed file");
    }

    fn directory(&self, package: &str, destination: &str) {
        self.store
            .connection
            .execute(
                "INSERT INTO managed_directories(game_path,root,path_key,relative_path,package_id)
                 VALUES (?1,'game',?2,?3,?4)",
                params![
                    self.game.to_string_lossy().as_ref(),
                    super::ownership_key(destination),
                    destination,
                    package
                ],
            )
            .expect("managed directory");
    }

    fn write(&self, destination: &str, bytes: &[u8]) {
        let path = self.game.join(destination);
        fs::create_dir_all(path.parent().expect("parent")).expect("parents");
        fs::write(path, bytes).expect("file");
    }

    fn plan(&self, files: &[(&str, u64)]) -> InstallPlan {
        InstallPlan {
            archive_blake3: "a".repeat(64),
            game_installation: self.game.clone(),
            status: PlanStatus::Reviewable,
            metadata_only: true,
            operations: files
                .iter()
                .map(|(destination, bytes)| PlannedFile {
                    source: format!("pack/{destination}"),
                    destination: (*destination).to_string(),
                    bytes: *bytes,
                    kind: "file-copy".into(),
                    owned_directory: None,
                    root: "game".into(),
                })
                .collect(),
            blockers: Vec::new(),
            skipped_generated_files: Vec::new(),
            content_base: None,
            content_root: None,
            overrides: Vec::new(),
        }
    }

    fn overrides(&self, files: &[(&str, u64)]) -> anyhow::Result<Vec<PlanOverride>> {
        self.store
            .claim_overrides(&self.game, &self.plan(files), None)
    }
}

#[test]
fn claim_overlap_between_profile_packages_is_allowed() {
    let fixture = Fixture::new();
    fixture.package("older", Some(0));
    fixture.file("older", SHARED, "old-digest", 3);
    let overrides = fixture.overrides(&[(SHARED, 7)]).expect("override");
    assert_eq!(overrides.len(), 1);
    let entry = &overrides[0];
    assert_eq!(entry.root, "game");
    assert_eq!(entry.destination, SHARED);
    assert_eq!(entry.replaced_package, "older");
    assert_eq!(entry.replaced_blake3, "old-digest");
    assert_eq!(entry.bytes, 7);
    assert_eq!(entry.archive_entry, format!("pack/{SHARED}"));
    assert_eq!(entry.package_id, None);
    assert_eq!(entry.blake3, None);
}

#[test]
fn claim_unmanaged_file_stays_a_conflict() {
    let fixture = Fixture::new();
    fixture.write(SHARED, b"user");
    assert_eq!(
        existing_destination_issue(&fixture.game, SHARED, false),
        Some("EXISTING_FILE")
    );
    assert_eq!(
        existing_destination_issue(&fixture.game, SHARED, true),
        None
    );
}

#[test]
fn claim_duplicate_in_one_plan_is_refused() {
    let fixture = Fixture::new();
    fixture.package("older", Some(0));
    fixture.file("older", SHARED, "old-digest", 3);
    let error = fixture
        .overrides(&[(SHARED, 3), (SHARED, 4)])
        .expect_err("duplicate");
    assert!(
        error.to_string().contains("DUPLICATE_DESTINATION"),
        "{error}"
    );
}

#[test]
fn claim_exclusive_directory_overlap_is_refused() {
    let fixture = Fixture::new();
    fixture.package("older", Some(0));
    fixture.directory("older", "addons");
    let error = fixture
        .overrides(&[("addons/one.plugin", 3)])
        .expect_err("exclusive directory");
    assert!(
        error
            .to_string()
            .contains("inside an exclusively owned directory"),
        "{error}"
    );
}

#[test]
fn claim_manifest_conflict_is_still_refused_with_overlapping_paths() {
    let fixture = Fixture::new();
    fixture.package("older", Some(0));
    fixture.file("older", SHARED, "old-digest", 3);
    let digest = "b".repeat(64);
    fixture
        .store
        .connection
        .execute(
            "INSERT INTO package_conflicts(package_id,bucket,game_id,mod_id,archive_blake3)
             VALUES ('older','deps','cyberpunk-2077','older',?1)",
            [&digest],
        )
        .expect("declared conflict");
    assert_eq!(fixture.overrides(&[(SHARED, 3)]).expect("override").len(), 1);
    assert_eq!(
        fixture
            .store
            .conflicting_packages(&fixture.game, None, &digest)
            .expect("conflicts"),
        ["older (archive hash)".to_string()]
    );
}

#[test]
fn claim_by_package_outside_the_active_profile_is_refused() {
    let fixture = Fixture::new();
    fixture.package("orphan", None);
    fixture.file("orphan", SHARED, "old-digest", 3);
    let error = fixture
        .overrides(&[(SHARED, 3)])
        .expect_err("inactive owner");
    assert!(
        error
            .to_string()
            .contains("claim owner is not in the active profile"),
        "{error}"
    );
}

#[test]
fn claim_update_skips_the_replaced_packages_own_claims() {
    let fixture = Fixture::new();
    fixture.package("older", Some(0));
    fixture.file("older", SHARED, "old-digest", 3);
    let overrides = fixture
        .store
        .claim_overrides(&fixture.game, &fixture.plan(&[(SHARED, 4)]), Some("older"))
        .expect("update");
    assert!(overrides.is_empty());
}

#[cfg(windows)]
#[test]
fn claim_same_file_spellings_resolve_to_one_override() {
    let fixture = Fixture::new();
    fixture.package("older", Some(0));
    fixture.file("older", "addons/One.Plugin", "old-digest", 3);
    let overrides = fixture
        .overrides(&[("ADDONS/one.plugin", 3)])
        .expect("case variant");
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].destination, "ADDONS/one.plugin");
}

#[test]
fn claim_symlink_at_the_destination_is_refused() {
    let fixture = Fixture::new();
    fixture.write("addons/real.plugin", b"real");
    let link = fixture.game.join(SHARED);
    fs::create_dir_all(link.parent().expect("parent")).expect("parents");
    if symlink_file(&fixture.game.join("addons/real.plugin"), &link).is_err() {
        return;
    }
    assert_eq!(
        existing_destination_issue(&fixture.game, SHARED, true),
        Some("LINKED_DESTINATION")
    );
}

#[test]
fn claim_directory_at_the_destination_is_refused() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.game.join(SHARED)).expect("directory");
    assert_eq!(
        existing_destination_issue(&fixture.game, SHARED, true),
        Some("EXISTING_FILE")
    );
}

#[test]
fn claim_plan_serializes_overrides_as_a_new_key() {
    let fixture = Fixture::new();
    fixture.package("older", Some(0));
    fixture.file("older", SHARED, "old-digest", 3);
    let mut plan = fixture.plan(&[(SHARED, 7)]);
    plan.overrides = fixture.overrides(&[(SHARED, 7)]).expect("override");
    let value = serde_json::to_value(&plan).expect("json");
    for key in [
        "archive_blake3",
        "game_installation",
        "status",
        "metadata_only",
        "operations",
        "blockers",
        "skipped_generated_files",
    ] {
        assert!(value.get(key).is_some(), "missing key {key}");
    }
    let overrides = value["overrides"].as_array().expect("overrides array");
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0]["destination"], SHARED);
    assert_eq!(overrides[0]["replaced_package"], "older");
    assert!(overrides[0].get("package_id").is_none());
}

#[test]
fn claim_second_plan_replaces_the_managed_file() {
    crate::test_support::seeded_roots(|roots| {
        let mut fixture = Fixture::new();
        let first = fixture.game.join("first.zip");
        zip_fixture(&first, SHARED, b"first");
        let inspection = crate::archive::inspect(&first).expect("inspect first");
        let plan = crate::planning::resolve(roots, "cyberpunk-2077", &inspection, &fixture.game)
            .expect("first plan");
        let installed = fixture
            .store
            .install_operations_zip(&fixture.game, &first, &plan)
            .expect("first install");
        let second = fixture.game.join("second.zip");
        zip_fixture(&second, SHARED, b"second");
        let inspection = crate::archive::inspect(&second).expect("inspect second");
        let plan = crate::planning::resolve_with_claims(
            &fixture.store,
            roots,
            "cyberpunk-2077",
            &inspection,
            &fixture.game,
            &[],
            &[],
            None,
            None,
        )
        .expect("second plan");
        assert_eq!(plan.status, PlanStatus::Reviewable, "{:?}", plan.blockers);
        assert_eq!(plan.overrides.len(), 1);
        assert_eq!(plan.overrides[0].destination, SHARED);
        assert_eq!(plan.overrides[0].replaced_package, installed.id);
        assert_eq!(
            plan.overrides[0].replaced_blake3,
            blake3::hash(b"first").to_hex().to_string()
        );
    });
}

#[cfg(windows)]
fn symlink_file(original: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(original, link)
}

#[cfg(not(windows))]
fn symlink_file(original: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(original, link)
}

fn zip_fixture(path: &Path, destination: &str, bytes: &[u8]) {
    use std::io::Write;
    let mut writer = zip::ZipWriter::new(fs::File::create(path).expect("archive"));
    writer
        .start_file(
            format!("pack/{destination}"),
            zip::write::SimpleFileOptions::default(),
        )
        .expect("entry");
    writer.write_all(bytes).expect("bytes");
    writer.finish().expect("finish");
}
