#[path = "support/main_bucket.rs"]
mod main_bucket;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

const SIGNED_KEY: &str = "40C666AA387404CE";
const REKEYED_KEY: &str = "EF1D3A7516E0AA02";

fn run(state: &Path, args: &[&str]) -> Output {
    main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .args([
            "--state-dir",
            state.to_str().expect("test fixture invariant"),
            "--json",
            "bucket",
        ])
        .args(args)
        .output()
        .expect("test fixture invariant")
}

fn run_plain(state: &Path, args: &[&str]) -> Output {
    main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .args([
            "--state-dir",
            state.to_str().expect("test fixture invariant"),
            "bucket",
        ])
        .args(args)
        .output()
        .expect("test fixture invariant")
}

fn git(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {arguments:?}: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git output")
        .trim()
        .to_owned()
}

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("fixture directory");
    for entry in fs::read_dir(source).expect("fixture entries") {
        let entry = entry.expect("fixture entry");
        let target = target.join(entry.file_name());
        if entry.file_type().expect("fixture type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::write(&target, fs::read(entry.path()).expect("fixture bytes"))
                .expect("fixture file");
        }
    }
}

fn catalog_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/signed-catalog")
        .join(name)
}

fn replace_catalog(repo: &Path, name: &str) {
    for entry in fs::read_dir(repo).expect("repository entries") {
        let entry = entry.expect("repository entry");
        if entry.file_name() == ".git" {
            continue;
        }
        if entry.file_type().expect("repository type").is_dir() {
            fs::remove_dir_all(entry.path()).expect("remove old catalog directory");
        } else {
            fs::remove_file(entry.path()).expect("remove old catalog file");
        }
    }
    copy_tree(&catalog_fixture(name), repo);
}

fn init_repo(root: &Path) -> PathBuf {
    let repo = root.join("signed-repo");
    fs::create_dir_all(&repo).expect("repository directory");
    git(&repo, &["init", "--initial-branch=main"]);
    repo
}

fn commit_all(repo: &Path, message: &str) -> String {
    git(repo, &["add", "-A"]);
    git(
        repo,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            message,
        ],
    );
    git(repo, &["rev-parse", "HEAD"])
}

fn sync(state: &Path, repo: &Path, explicit_source: bool) -> Output {
    let repo = repo.to_str().expect("repository path");
    if explicit_source {
        run(
            state,
            &["sync", "community", "--source", repo, "--revision", "HEAD"],
        )
    } else {
        run(state, &["sync", "community"])
    }
}

fn official_count() -> usize {
    let games = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main/games");
    fs::read_dir(games)
        .expect("games directory")
        .map(|game| {
            fs::read_dir(game.expect("game entry").path())
                .expect("packages")
                .filter(|entry| {
                    let path = entry.as_ref().expect("fixture invariant").path();
                    path.extension().is_some_and(|e| e == "json")
                        && path.file_name().expect("fixture invariant") != "module.json"
                })
                .count()
        })
        .sum()
}

fn catalog_search(state: &Path) -> Output {
    main_bucket::command(env!("CARGO_BIN_EXE_modn"))
        .arg("--state-dir")
        .arg(state)
        .args(["--json", "search"])
        .output()
        .expect("search")
}

fn seed(root: &Path, body: &str) {
    let directory = root.join("games/test-game");
    fs::create_dir_all(&directory).expect("test fixture invariant");
    fs::write(
        root.join("bucket.json"),
        r#"{"schema_version":1,"name":"community","title":"Community"}"#,
    )
    .expect("test fixture invariant");
    fs::write(directory.join("example.json"), body).expect("test fixture invariant");
}

fn valid_manifest() -> String {
    serde_json::json!({
        "schema_version":1,"id":"example","game":"test-game","name":"Example",
        "version":"1.0","archive_format":"zip",
        "sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "source":{"kind":"manual","filename":"example.zip"},"dependencies":[]
    })
    .to_string()
}

fn output(result: &Output) -> String {
    String::from_utf8_lossy(&result.stderr).to_string()
}

#[test]
fn bundled_main_is_available_without_mutating_state() {
    let temp = TempDir::new().expect("test fixture invariant");
    let result = run(temp.path(), &["list"]);
    assert!(result.status.success(), "{}", output(&result));
    let entries: serde_json::Value =
        serde_json::from_slice(&result.stdout).expect("test fixture invariant");
    assert_eq!(entries.as_array().expect("test fixture invariant").len(), 1);
    assert_eq!(entries[0]["name"], "main");
    assert!(!temp.path().join("buckets").exists());
    assert!(!temp.path().join("state.sqlite").exists());
    let result = run(temp.path(), &["show", "main/cyberpunk-2077/red4ext"]);
    assert!(result.status.success(), "{}", output(&result));
    let entry: serde_json::Value =
        serde_json::from_slice(&result.stdout).expect("test fixture invariant");
    assert_eq!(entry["version"], "1.31.0");
    assert_eq!(entry["source"]["kind"], "https");
    assert!(!temp.path().join("state.sqlite").exists());
}

#[test]
fn local_registration_resolves_and_rejects_duplicate_registration() {
    let temp = TempDir::new().expect("test fixture invariant");
    let bucket = temp.path().join("source");
    seed(&bucket, &valid_manifest());
    let state = temp.path().join("state");
    let add = run(
        &state,
        &[
            "add",
            "community",
            bucket.to_str().expect("test fixture invariant"),
        ],
    );
    assert!(add.status.success(), "{}", output(&add));
    let listed = run(&state, &["list"]);
    assert!(listed.status.success(), "{}", output(&listed));
    let entries: serde_json::Value =
        serde_json::from_slice(&listed.stdout).expect("test fixture invariant");
    let local = entries
        .as_array()
        .expect("test fixture invariant")
        .iter()
        .find(|entry| entry["name"] == "community")
        .expect("registered local bucket");
    assert!(local.get("signer_key_id").is_none());
    let search = catalog_search(&state);
    assert!(search.status.success(), "{}", output(&search));
    let entries: serde_json::Value =
        serde_json::from_slice(&search.stdout).expect("test fixture invariant");
    assert_eq!(
        entries.as_array().expect("test fixture invariant").len(),
        official_count() + 1
    );
    let show = run(&state, &["show", "community/test-game/example"]);
    assert!(show.status.success(), "{}", output(&show));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&show.stdout).expect("test fixture invariant")
            ["id"],
        "example"
    );
    assert!(!run(
        &state,
        &[
            "add",
            "community",
            bucket.to_str().expect("test fixture invariant")
        ]
    )
    .status
    .success());
    let removed = run(&state, &["remove", "community"]);
    assert!(removed.status.success(), "{}", output(&removed));
    assert!(bucket.join("bucket.json").exists());
    let search = catalog_search(&state);
    assert!(search.status.success(), "{}", output(&search));
    let after: serde_json::Value =
        serde_json::from_slice(&search.stdout).expect("test fixture invariant");
    assert_eq!(after.as_array().expect("array").len(), official_count());
    assert!(!run(&state, &["remove", "main"]).status.success());
    assert!(!state.join("state.sqlite").exists());
}

#[test]
fn signed_git_catalog_sync_pins_and_lists_the_signer() {
    let temp = TempDir::new().expect("test fixture invariant");
    let repo = init_repo(temp.path());
    replace_catalog(&repo, "v1");
    let commit = commit_all(&repo, "v1");
    let state = temp.path().join("state");

    let synced = sync(&state, &repo, true);
    assert!(synced.status.success(), "{}", output(&synced));
    let value: serde_json::Value = serde_json::from_slice(&synced.stdout).expect("sync JSON");
    assert_eq!(value["commit"], commit);
    assert_eq!(value["signer"]["key_id"], SIGNED_KEY);
    assert!(
        output(&synced).contains(&format!(
            "Pinned signer key {SIGNED_KEY} for bucket community; compare this fingerprint with the catalog author."
        )),
        "{}",
        output(&synced)
    );

    let listed = run(&state, &["list"]);
    assert!(listed.status.success(), "{}", output(&listed));
    let entries: serde_json::Value = serde_json::from_slice(&listed.stdout).expect("list JSON");
    let entry = entries
        .as_array()
        .expect("list array")
        .iter()
        .find(|entry| entry["name"] == "community")
        .expect("registered Git bucket");
    assert_eq!(entry["signer_key_id"], SIGNED_KEY);

    let plain = run_plain(&state, &["list"]);
    assert!(plain.status.success(), "{}", output(&plain));
    assert!(
        String::from_utf8_lossy(&plain.stdout).contains(SIGNED_KEY),
        "{}",
        String::from_utf8_lossy(&plain.stdout)
    );
}

#[test]
fn later_revision_signed_by_the_pinned_key_succeeds() {
    let temp = TempDir::new().expect("test fixture invariant");
    let repo = init_repo(temp.path());
    replace_catalog(&repo, "v1");
    commit_all(&repo, "v1");
    let state = temp.path().join("state");
    let first = sync(&state, &repo, true);
    assert!(first.status.success(), "{}", output(&first));

    replace_catalog(&repo, "v2");
    let second = commit_all(&repo, "v2");
    let synced = sync(&state, &repo, false);
    assert!(synced.status.success(), "{}", output(&synced));
    let value: serde_json::Value = serde_json::from_slice(&synced.stdout).expect("sync JSON");
    assert_eq!(value["commit"], second);
    assert_eq!(value["signer"]["key_id"], SIGNED_KEY);
    assert!(
        !output(&synced).contains("Pinned signer key"),
        "{}",
        output(&synced)
    );
}

#[test]
fn changed_signer_key_is_refused_and_preserves_the_published_copy() {
    let temp = TempDir::new().expect("test fixture invariant");
    let repo = init_repo(temp.path());
    replace_catalog(&repo, "v1");
    commit_all(&repo, "v1");
    let state = temp.path().join("state");
    let synced = sync(&state, &repo, true);
    assert!(synced.status.success(), "{}", output(&synced));
    let registration = state.join("buckets/registrations/community.json");
    let before_registration = fs::read(&registration).expect("registration bytes");
    let pointer: serde_json::Value =
        serde_json::from_slice(&before_registration).expect("registration JSON");
    let published = Path::new(pointer["path"].as_str().expect("published path"));
    let package = published.join("games/test-game/example.json");
    let before_package = fs::read(&package).expect("published package");

    replace_catalog(&repo, "v2-rekeyed");
    commit_all(&repo, "rekeyed");
    let refused = sync(&state, &repo, false);
    assert!(!refused.status.success());
    assert!(
        output(&refused).contains(&format!(
            "Signer key changed from {SIGNED_KEY} to {REKEYED_KEY} for bucket community. Modn does not rotate keys yet (#122); remove the bucket and sync it again to trust the new key."
        )),
        "{}",
        output(&refused)
    );
    assert_eq!(
        fs::read(&registration).expect("registration"),
        before_registration
    );
    assert_eq!(fs::read(&package).expect("package"), before_package);
}

#[test]
fn unsigned_and_invalid_revisions_are_refused_at_first_and_later_sync() {
    let temp = TempDir::new().expect("test fixture invariant");
    let repo = init_repo(temp.path());
    let registration = temp
        .path()
        .join("state/buckets/registrations/community.json");
    let state = temp.path().join("state");

    replace_catalog(&repo, "v1");
    fs::remove_file(repo.join("catalog.pub")).expect("remove public key");
    fs::remove_file(repo.join("catalog.sig")).expect("remove signature");
    commit_all(&repo, "unsigned");
    let refused = sync(&state, &repo, true);
    assert!(!refused.status.success());
    assert!(
        output(&refused)
            .contains("Git catalog community at HEAD is unsigned; Modn refuses unsigned catalogs."),
        "{}",
        output(&refused)
    );
    assert!(!registration.exists());

    replace_catalog(&repo, "v1");
    let manifest = repo.join("games/test-game/example.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).expect("manifest")).expect("manifest JSON");
    value["version"] = serde_json::json!("9.0");
    fs::write(&manifest, serde_json::to_vec(&value).expect("manifest")).expect("manifest");
    commit_all(&repo, "tampered");
    let refused = sync(&state, &repo, true);
    assert!(!refused.status.success());
    assert!(
        output(&refused).contains("catalog signature verification failed"),
        "{}",
        output(&refused)
    );
    assert!(!registration.exists());

    replace_catalog(&repo, "v1");
    commit_all(&repo, "valid");
    let synced = sync(&state, &repo, true);
    assert!(synced.status.success(), "{}", output(&synced));
    let before_registration = fs::read(&registration).expect("registration bytes");
    let pointer: serde_json::Value =
        serde_json::from_slice(&before_registration).expect("registration JSON");
    let published = Path::new(pointer["path"].as_str().expect("published path"));
    let package = published.join("games/test-game/example.json");
    let before_package = fs::read(&package).expect("published package");

    fs::remove_file(repo.join("catalog.pub")).expect("remove public key");
    fs::remove_file(repo.join("catalog.sig")).expect("remove signature");
    commit_all(&repo, "unsigned update");
    let refused = sync(&state, &repo, false);
    assert!(!refused.status.success());
    assert!(
        output(&refused).contains("is unsigned"),
        "{}",
        output(&refused)
    );
    assert_eq!(
        fs::read(&registration).expect("registration"),
        before_registration
    );
    assert_eq!(fs::read(&package).expect("package"), before_package);

    replace_catalog(&repo, "v2");
    commit_all(&repo, "valid update");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).expect("manifest")).expect("manifest JSON");
    value["version"] = serde_json::json!("9.0");
    fs::write(&manifest, serde_json::to_vec(&value).expect("manifest")).expect("manifest");
    commit_all(&repo, "tampered update");
    let refused = sync(&state, &repo, false);
    assert!(!refused.status.success());
    assert!(
        output(&refused).contains("catalog signature verification failed"),
        "{}",
        output(&refused)
    );
    assert_eq!(
        fs::read(&registration).expect("registration"),
        before_registration
    );
    assert_eq!(fs::read(&package).expect("package"), before_package);
}

#[test]
fn registration_before_catalog_signing_is_refused_until_removed() {
    let temp = TempDir::new().expect("test fixture invariant");
    let repo = init_repo(temp.path());
    replace_catalog(&repo, "v1");
    commit_all(&repo, "v1");
    let state = temp.path().join("state");
    let synced = sync(&state, &repo, true);
    assert!(synced.status.success(), "{}", output(&synced));

    let registration = state.join("buckets/registrations/community.json");
    let mut pointer: serde_json::Value =
        serde_json::from_slice(&fs::read(&registration).expect("registration")).expect("JSON");
    pointer["git"]
        .as_object_mut()
        .expect("Git registration")
        .remove("signer");
    fs::write(&registration, serde_json::to_vec(&pointer).expect("JSON")).expect("registration");
    let message =
        "Bucket community was registered before catalog signing; remove it and sync it again.";

    let listed = run(&state, &["list"]);
    assert!(!listed.status.success());
    assert!(output(&listed).contains(message), "{}", output(&listed));
    let refused = sync(&state, &repo, false);
    assert!(!refused.status.success());
    assert!(output(&refused).contains(message), "{}", output(&refused));

    let removed = run(&state, &["remove", "community"]);
    assert!(removed.status.success(), "{}", output(&removed));
    let synced = sync(&state, &repo, true);
    assert!(synced.status.success(), "{}", output(&synced));
    assert!(
        output(&synced).contains("Pinned signer key"),
        "{}",
        output(&synced)
    );
}

#[test]
fn malformed_packages_are_rejected_before_registration() {
    let temp = TempDir::new().expect("test fixture invariant");
    let bucket = temp.path().join("source");
    let state = temp.path().join("state");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&valid_manifest()).expect("test fixture invariant");
    for (field, value) in [
        ("sha256", serde_json::json!("bad")),
        ("id", serde_json::json!("../escape")),
        ("dependencies", serde_json::json!(["example"])),
        (
            "source",
            serde_json::json!({"kind":"manual","filename":"../bad.zip"}),
        ),
        ("schema_version", serde_json::json!(2)),
    ] {
        let original = manifest[field].clone();
        manifest[field] = value;
        seed(&bucket, &manifest.to_string());
        let result = run(
            &state,
            &[
                "add",
                "community",
                bucket.to_str().expect("test fixture invariant"),
            ],
        );
        assert!(!result.status.success(), "{field}");
        assert!(!state.join("buckets").exists(), "{field}");
        manifest[field] = original;
    }
}

#[test]
fn registered_bucket_corruption_never_silently_falls_back() {
    let temp = TempDir::new().expect("test fixture invariant");
    let bucket = temp.path().join("source");
    let state = temp.path().join("state");
    seed(&bucket, &valid_manifest());
    assert!(run(
        &state,
        &[
            "add",
            "community",
            bucket.to_str().expect("test fixture invariant")
        ]
    )
    .status
    .success());
    fs::write(bucket.join("games/test-game/example.json"), b"{bad")
        .expect("test fixture invariant");
    let failed = catalog_search(&state);
    assert!(!failed.status.success());
    assert!(output(&failed).contains("invalid bucket JSON"));
    assert!(!state.join("state.sqlite").exists());
}

#[cfg(unix)]
#[test]
fn linked_manifest_is_refused() {
    use std::os::unix::fs::symlink;
    let temp = TempDir::new().expect("test fixture invariant");
    let bucket = temp.path().join("source");
    seed(&bucket, &valid_manifest());
    let manifest = bucket.join("games/test-game/example.json");
    let other = temp.path().join("outside.json");
    fs::rename(&manifest, &other).expect("test fixture invariant");
    symlink(&other, &manifest).expect("test fixture invariant");
    assert!(!run(
        &temp.path().join("state"),
        &[
            "add",
            "community",
            bucket.to_str().expect("test fixture invariant")
        ]
    )
    .status
    .success());
}

#[test]
fn leftover_temp_files_outside_registry_do_not_block_reads() {
    let temp = TempDir::new().expect("test fixture invariant");
    let bucket = temp.path().join("source");
    let state = temp.path().join("state");
    seed(&bucket, &valid_manifest());
    assert!(run(
        &state,
        &["add", "community", bucket.to_str().expect("path")]
    )
    .status
    .success());
    let leftover = state.join("buckets").join(".orphaned-staging");
    fs::write(&leftover, b"incomplete registration").expect("orphan");
    let found = catalog_search(&state);
    assert!(found.status.success(), "{}", output(&found));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&found.stdout)
            .expect("JSON")
            .as_array()
            .expect("array")
            .len(),
        official_count() + 1
    );
    assert!(run(&state, &["remove", "community"]).status.success());
    assert!(
        leftover.exists(),
        "registry removal must not claim stray files"
    );
}

#[test]
fn linked_bucket_registry_parent_cannot_redirect_registration_writes() {
    let temp = TempDir::new().expect("test fixture invariant");
    let source = temp.path().join("source");
    seed(&source, &valid_manifest());
    let state = temp.path().join("state");
    let outside = temp.path().join("external-target");
    fs::create_dir_all(&state).expect("state");
    fs::create_dir_all(&outside).expect("outside");
    let link = state.join("buckets");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &link).expect("link");
    #[cfg(windows)]
    {
        let result = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .output()
            .expect("create junction");
        assert!(result.status.success(), "{}", output(&result));
    }
    let result = run(
        &state,
        &["add", "community", source.to_str().expect("path")],
    );
    assert!(
        !result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    assert!(fs::read_dir(&outside).expect("outside").next().is_none());
}

#[test]
fn linked_state_root_cannot_redirect_registration_writes() {
    let temp = TempDir::new().expect("test fixture invariant");
    let source = temp.path().join("source");
    seed(&source, &valid_manifest());
    let real_state = temp.path().join("real-state");
    let linked_state = temp.path().join("linked-state");
    fs::create_dir_all(&real_state).expect("real state");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real_state, &linked_state).expect("link");
    #[cfg(windows)]
    {
        let result = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&linked_state)
            .arg(&real_state)
            .output()
            .expect("create junction");
        assert!(result.status.success(), "{}", output(&result));
    }
    let result = run(
        &linked_state,
        &["add", "community", source.to_str().expect("path")],
    );
    assert!(
        !result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    assert!(fs::read_dir(&real_state)
        .expect("real state")
        .next()
        .is_none());
}

#[cfg(unix)]
#[test]
fn named_pipe_metadata_is_rejected_without_opening_it() {
    let temp = TempDir::new().expect("test fixture invariant");
    let bucket = temp.path().join("source");
    seed(&bucket, &valid_manifest());
    let path = bucket.join("games/test-game/example.json");
    fs::remove_file(&path).expect("remove fixture");
    let creation = Command::new("mkfifo").arg(&path).output().expect("mkfifo");
    assert!(
        creation.status.success(),
        "{}",
        String::from_utf8_lossy(&creation.stderr)
    );
    let result = run(
        &temp.path().join("state"),
        &["add", "community", bucket.to_str().expect("path")],
    );
    assert!(!result.status.success());
    assert!(output(&result).contains("regular file"));
}

#[test]
fn bucket_registration_does_not_write_into_detected_game_root() {
    let temp = TempDir::new().expect("test fixture invariant");
    let bucket = temp.path().join("source");
    seed(&bucket, &valid_manifest());
    let game = temp.path().join("game");
    let marker = game.join("bin/x64/Cyberpunk2077.exe");
    fs::create_dir_all(marker.parent().expect("marker parent")).expect("game");
    fs::write(&marker, b"unmodified game marker").expect("marker");
    let result = run(&game, &["add", "community", bucket.to_str().expect("path")]);
    assert!(
        !result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    assert!(
        output(&result).contains("outside a game installation"),
        "{}",
        output(&result)
    );
    assert!(!game.join("buckets").exists());
    assert_eq!(
        fs::read(&marker).expect("marker"),
        b"unmodified game marker"
    );
}

#[test]
fn bucket_title_control_characters_are_rejected() {
    let temp = TempDir::new().expect("test fixture invariant");
    let bucket = temp.path().join("source");
    seed(&bucket, &valid_manifest());
    fs::write(
        bucket.join("bucket.json"),
        r#"{"schema_version":1,"name":"community","title":"Community\u001b[2J"}"#,
    )
    .expect("metadata");
    let state = temp.path().join("state");
    let result = run(
        &state,
        &["add", "community", bucket.to_str().expect("path")],
    );
    assert!(
        !result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    assert!(!state.join("buckets").exists());
}

#[test]
fn bucket_registry_destination_inside_game_root_is_refused() {
    let temp = TempDir::new().expect("test fixture invariant");
    let source = temp.path().join("source");
    seed(&source, &valid_manifest());
    let state = temp.path().join("state");
    let game = state.join("buckets");
    let marker = game.join("bin/x64/Cyberpunk2077.exe");
    fs::create_dir_all(marker.parent().expect("marker parent")).expect("fixture game");
    fs::write(&marker, b"original marker").expect("fixture game marker");
    let result = run(
        &state,
        &["add", "community", source.to_str().expect("source path")],
    );
    assert!(!result.status.success(), "{}", output(&result));
    assert!(
        output(&result).contains("outside a game installation"),
        "{}",
        output(&result)
    );
    assert!(!game.join("registrations").exists());
    assert_eq!(fs::read(&marker).expect("marker"), b"original marker");
    assert!(!state.join("state.sqlite").exists());
}

#[test]
fn obsolete_module_directory_is_not_a_valid_bucket() {
    let temp = TempDir::new().expect("fixture");
    let bucket = temp.path().join("source");
    let state = temp.path().join("state");
    seed(&bucket, &valid_manifest());
    fs::create_dir_all(bucket.join("modules/test-game")).expect("fixture");
    let result = run(
        &state,
        &["add", "community", bucket.to_str().expect("path")],
    );
    assert!(!result.status.success());
    assert!(output(&result).contains("bucket module directories are unsupported"));
    assert!(!state.join("buckets").exists());
    assert!(!run(&state, &["modules"]).status.success());
}

#[test]
fn package_catalog_remains_independent_of_game_module_source() {
    let temp = TempDir::new().expect("fixture");
    let state = temp.path().join("state");
    let result = run(&state, &["show", "main/cyberpunk-2077/cet"]);
    assert!(result.status.success(), "{}", output(&result));
    let entry: serde_json::Value = serde_json::from_slice(&result.stdout).expect("json");
    assert_eq!(entry["id"], "cet");
    assert!(!state.exists());
}

#[path = "support/bucket_safety.rs"]
mod extra_cases;
