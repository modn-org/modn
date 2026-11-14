//! Author tools: key generation, signing and verification at the CLI.

#[path = "support/catalog_author.rs"]
mod catalog_author;
#[path = "support/catalog_authoring.rs"]
mod catalog_authoring;
#[path = "support/main_bucket.rs"]
mod main_bucket;

use catalog_author::{keygen, sign, verify};
use catalog_authoring::{
    commit_all, copy_tree, fingerprint, init_repo, json, path_text, prepare_catalog, run, snapshot,
    stderr, stdout,
};
use std::fs;
use tempfile::TempDir;

const SIGNATURE: &str = "catalog.sig";
const PUBLIC_KEY: &str = "catalog.pub";

#[test]
fn keygen_sign_verify_round_trip_pins_the_fingerprint_at_sync() {
    let temp = TempDir::new().expect("test fixture invariant");
    let catalog = prepare_catalog(temp.path(), "catalog");
    let keys = temp.path().join("keys");

    let generated = keygen(&keys, "author");
    assert!(generated.status.success(), "{}", stderr(&generated));
    let pinned = fingerprint(&generated);
    assert_eq!(pinned.len(), 16, "{pinned}");
    assert!(pinned.chars().all(|c| c.is_ascii_hexdigit()), "{pinned}");

    let signed = sign(&catalog, &keys.join("author.key"));
    assert!(signed.status.success(), "{}", stderr(&signed));
    let signed_json = json(&signed);
    assert_eq!(signed_json["fingerprint"], pinned.as_str());
    assert!(signed_json["digest"]
        .as_str()
        .expect("digest")
        .starts_with("blake3:"));
    assert_eq!(
        fs::read(catalog.join(PUBLIC_KEY)).expect("catalog public key"),
        fs::read(keys.join("author.pub")).expect("generated public key")
    );

    let verified = verify(&catalog, &catalog.join(PUBLIC_KEY));
    assert!(verified.status.success(), "{}", stderr(&verified));
    let verified_json = json(&verified);
    assert_eq!(verified_json["fingerprint"], pinned.as_str());
    assert_eq!(verified_json["digest"], signed_json["digest"]);

    let repo = init_repo(temp.path());
    copy_tree(&catalog, &repo);
    let commit = commit_all(&repo, "signed catalog");
    let state = temp.path().join("state");
    let synced = run(&[
        "--state-dir",
        path_text(&state),
        "--json",
        "bucket",
        "sync",
        "community",
        "--source",
        path_text(&repo),
        "--revision",
        "HEAD",
    ]);
    assert!(synced.status.success(), "{}", stderr(&synced));
    let synced_json = json(&synced);
    assert_eq!(synced_json["commit"], commit.as_str());
    assert_eq!(synced_json["signer"]["key_id"], pinned.as_str());
    assert!(
        stderr(&synced).contains(&format!(
            "Pinned signer key {pinned} for bucket community; compare this fingerprint with the catalog author."
        )),
        "{}",
        stderr(&synced)
    );
}

#[test]
fn sign_and_verify_work_in_a_git_checkout_and_sync_accepts_it() {
    let temp = TempDir::new().expect("test fixture invariant");
    let repo = init_repo(temp.path());
    copy_tree(&catalog_authoring::catalog_fixture("v1"), &repo);
    assert!(repo.join(".git").exists());
    let keys = temp.path().join("keys");
    let generated = keygen(&keys, "author");
    assert!(generated.status.success(), "{}", stderr(&generated));
    let pinned = fingerprint(&generated);

    let signed = sign(&repo, &keys.join("author.key"));
    assert!(signed.status.success(), "{}", stderr(&signed));
    let verified = verify(&repo, &repo.join(PUBLIC_KEY));
    assert!(verified.status.success(), "{}", stderr(&verified));
    assert_eq!(json(&verified)["fingerprint"], pinned.as_str());

    commit_all(&repo, "signed catalog in a checkout");
    let state = temp.path().join("state");
    let synced = run(&[
        "--state-dir",
        path_text(&state),
        "--json",
        "bucket",
        "sync",
        "community",
        "--source",
        path_text(&repo),
        "--revision",
        "HEAD",
    ]);
    assert!(synced.status.success(), "{}", stderr(&synced));
    assert_eq!(json(&synced)["signer"]["key_id"], pinned.as_str());
}

#[test]
fn verify_refuses_changed_added_and_deleted_files_and_writes_nothing() {
    let temp = TempDir::new().expect("test fixture invariant");
    let catalog = prepare_catalog(temp.path(), "catalog");
    let keys = temp.path().join("keys");
    assert!(keygen(&keys, "author").status.success());
    let signed = sign(&catalog, &keys.join("author.key"));
    assert!(signed.status.success(), "{}", stderr(&signed));
    let state = temp.path().join("state");

    fs::write(catalog.join("mods/test-game/example.json"), b"{}").expect("modified file");
    let before = snapshot(&catalog);
    let changed = run(&[
        "--state-dir",
        path_text(&state),
        "catalog",
        "verify",
        path_text(&catalog),
        "--key",
        path_text(&catalog.join(PUBLIC_KEY)),
    ]);
    assert!(!changed.status.success());
    assert!(
        stderr(&changed).contains("does not verify"),
        "{}",
        stderr(&changed)
    );
    assert!(
        stderr(&changed).contains("changed, added or deleted file"),
        "{}",
        stderr(&changed)
    );
    assert_eq!(snapshot(&catalog), before);
    assert!(!state.exists(), "verify wrote manager state");

    fs::write(catalog.join("mods/test-game/added.json"), b"{}").expect("added file");
    let before = snapshot(&catalog);
    let added = verify(&catalog, &catalog.join(PUBLIC_KEY));
    assert!(!added.status.success());
    assert!(
        stderr(&added).contains("does not verify"),
        "{}",
        stderr(&added)
    );
    assert_eq!(snapshot(&catalog), before);

    fs::remove_file(catalog.join("mods/test-game/added.json")).expect("remove added file");
    fs::remove_file(catalog.join("mods/test-game/example.json")).expect("deleted file");
    let before = snapshot(&catalog);
    let deleted = verify(&catalog, &catalog.join(PUBLIC_KEY));
    assert!(!deleted.status.success());
    assert!(
        stderr(&deleted).contains("does not verify"),
        "{}",
        stderr(&deleted)
    );
    assert_eq!(snapshot(&catalog), before);
}

#[test]
fn verify_names_both_fingerprints_and_reports_a_missing_signature() {
    let temp = TempDir::new().expect("test fixture invariant");
    let catalog = prepare_catalog(temp.path(), "catalog");
    let keys = temp.path().join("keys");
    let generated = keygen(&keys, "author");
    assert!(generated.status.success(), "{}", stderr(&generated));
    let expected = fingerprint(&generated);
    let signed = sign(&catalog, &keys.join("author.key"));
    assert!(signed.status.success(), "{}", stderr(&signed));

    let other = temp.path().join("other");
    let generated_other = keygen(&other, "other");
    assert!(
        generated_other.status.success(),
        "{}",
        stderr(&generated_other)
    );
    let found_by_other = fingerprint(&generated_other);
    assert_ne!(expected, found_by_other);
    let before = snapshot(&catalog);
    let wrong = verify(&catalog, &other.join("other.pub"));
    assert!(!wrong.status.success());
    assert!(stderr(&wrong).contains(&expected), "{}", stderr(&wrong));
    assert!(
        stderr(&wrong).contains(&found_by_other),
        "{}",
        stderr(&wrong)
    );
    assert_eq!(snapshot(&catalog), before);

    fs::remove_file(catalog.join(SIGNATURE)).expect("removed signature");
    let before = snapshot(&catalog);
    let missing = verify(&catalog, &other.join("other.pub"));
    assert!(!missing.status.success());
    assert!(
        stderr(&missing).contains("catalog signature is missing"),
        "{}",
        stderr(&missing)
    );
    assert_eq!(snapshot(&catalog), before);
}

#[test]
fn keygen_never_prints_the_secret_and_refuses_to_overwrite() {
    let temp = TempDir::new().expect("test fixture invariant");
    let keys = temp.path().join("keys");
    let generated = run(&[
        "catalog",
        "keygen",
        "--out",
        path_text(&keys),
        "--name",
        "author",
    ]);
    assert!(generated.status.success(), "{}", stderr(&generated));
    let printed = stdout(&generated);
    let word = printed
        .lines()
        .find(|line| line.starts_with("Signing key fingerprint "))
        .expect("fingerprint line");
    let pinned = word
        .trim_start_matches("Signing key fingerprint ")
        .to_owned();
    assert_eq!(pinned.len(), 16, "{pinned}");
    assert!(
        printed.contains(path_text(&keys.join("author.pub"))),
        "{printed}"
    );

    let secret = fs::read_to_string(keys.join("author.key")).expect("secret key file");
    let body = secret
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .expect("secret key body");
    assert!(!body.starts_with("untrusted comment:"));
    assert!(body.len() > 100, "secret body looks empty");
    assert!(!printed.contains(body), "stdout leaked the secret key");
    assert!(
        !stderr(&generated).contains(body),
        "stderr leaked the secret key"
    );

    let secret_bytes = fs::read(keys.join("author.key")).expect("secret key bytes");
    let public_bytes = fs::read(keys.join("author.pub")).expect("public key bytes");
    let repeated = run(&[
        "catalog",
        "keygen",
        "--out",
        path_text(&keys),
        "--name",
        "author",
    ]);
    assert!(!repeated.status.success());
    assert!(
        stderr(&repeated).contains("signing key already exists"),
        "{}",
        stderr(&repeated)
    );
    assert_eq!(
        fs::read(keys.join("author.key")).expect("secret key bytes"),
        secret_bytes
    );
    assert_eq!(
        fs::read(keys.join("author.pub")).expect("public key bytes"),
        public_bytes
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(keys.join("author.key"))
            .expect("secret key metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "secret key mode is {mode:o}");
    }
}

#[test]
fn sign_refuses_unsafe_inputs_and_keeps_the_previous_signature() {
    let temp = TempDir::new().expect("test fixture invariant");
    let catalog = prepare_catalog(temp.path(), "catalog");
    let keys = temp.path().join("keys");
    assert!(keygen(&keys, "author").status.success());
    let signed = sign(&catalog, &keys.join("author.key"));
    assert!(signed.status.success(), "{}", stderr(&signed));
    let signature_bytes = fs::read(catalog.join(SIGNATURE)).expect("signature bytes");

    let not_directory = run(&[
        "catalog",
        "sign",
        path_text(&catalog.join("bucket.json")),
        "--key",
        path_text(&keys.join("author.key")),
    ]);
    assert!(!not_directory.status.success());
    assert!(
        stderr(&not_directory).contains("is not a directory"),
        "{}",
        stderr(&not_directory)
    );
    assert_eq!(
        fs::read(catalog.join(SIGNATURE)).expect("signature bytes"),
        signature_bytes
    );

    assert!(keygen(&catalog, "inside").status.success());
    let inside = run(&[
        "catalog",
        "sign",
        path_text(&catalog),
        "--key",
        path_text(&catalog.join("inside.key")),
    ]);
    assert!(!inside.status.success());
    assert!(
        stderr(&inside).contains("must live outside the catalog directory"),
        "{}",
        stderr(&inside)
    );
    assert_eq!(
        fs::read(catalog.join(SIGNATURE)).expect("signature bytes"),
        signature_bytes
    );
    assert_eq!(
        fs::read(catalog.join(PUBLIC_KEY)).expect("public key bytes"),
        fs::read(keys.join("author.pub")).expect("generated public key")
    );
}
