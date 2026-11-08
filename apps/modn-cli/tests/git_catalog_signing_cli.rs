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

fn output(result: &Output) -> String {
    String::from_utf8_lossy(&result.stderr).to_string()
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
