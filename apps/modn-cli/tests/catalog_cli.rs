//! Author tools: key generation, signing, verification, archive pins and Git sync.

#[path = "support/catalog_authoring.rs"]
mod catalog_authoring;
#[path = "support/main_bucket.rs"]
mod main_bucket;

use catalog_authoring::{
    commit_all, copy_tree, fingerprint, init_repo, json, path_text, prepare_catalog, run, run_with,
    snapshot, stderr, stdout,
};
use serde_json::Value;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

const SIGNATURE: &str = "catalog.sig";
const PUBLIC_KEY: &str = "catalog.pub";

fn keygen(keys: &Path, name: &str) -> std::process::Output {
    run(&[
        "--json",
        "catalog",
        "keygen",
        "--out",
        path_text(keys),
        "--name",
        name,
    ])
}

fn sign(catalog: &Path, key: &Path) -> std::process::Output {
    run(&[
        "--json",
        "catalog",
        "sign",
        path_text(catalog),
        "--key",
        path_text(key),
    ])
}

fn verify(catalog: &Path, key: &Path) -> std::process::Output {
    run(&[
        "--json",
        "catalog",
        "verify",
        path_text(catalog),
        "--key",
        path_text(key),
    ])
}

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
    assert!(stderr(&changed).contains("does not verify"), "{}", stderr(&changed));
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
    assert!(stderr(&added).contains("does not verify"), "{}", stderr(&added));
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
    assert!(generated_other.status.success(), "{}", stderr(&generated_other));
    let found_by_other = fingerprint(&generated_other);
    assert_ne!(expected, found_by_other);
    let before = snapshot(&catalog);
    let wrong = verify(&catalog, &other.join("other.pub"));
    assert!(!wrong.status.success());
    assert!(stderr(&wrong).contains(&expected), "{}", stderr(&wrong));
    assert!(stderr(&wrong).contains(&found_by_other), "{}", stderr(&wrong));
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
    assert!(printed.contains(path_text(&keys.join("author.pub"))), "{printed}");

    let secret = fs::read_to_string(keys.join("author.key")).expect("secret key file");
    let body = secret
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .next_back()
        .expect("secret key body");
    assert!(!body.starts_with("untrusted comment:"));
    assert!(body.len() > 100, "secret body looks empty");
    assert!(!printed.contains(body), "stdout leaked the secret key");
    assert!(!stderr(&generated).contains(body), "stderr leaked the secret key");

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

#[test]
fn encrypted_key_signs_with_its_password_and_refuses_without_it() {
    let temp = TempDir::new().expect("test fixture invariant");
    let catalog = prepare_catalog(temp.path(), "catalog");
    let keys = temp.path().join("encrypted");
    let variable = "MODN_TEST_CATALOG_KEY_PASSWORD";
    let generated = run_with(
        &[
            "catalog",
            "keygen",
            "--out",
            path_text(&keys),
            "--name",
            "author",
            "--password-env",
            variable,
        ],
        &[(variable, "correct horse battery")],
    );
    assert!(generated.status.success(), "{}", stderr(&generated));
    let key = keys.join("author.key");
    assert!(fs::read_to_string(&key)
        .expect("secret key")
        .lines()
        .count()
        >= 2);

    let without = run(&[
        "catalog",
        "sign",
        path_text(&catalog),
        "--key",
        path_text(&key),
    ]);
    assert!(!without.status.success());
    assert!(stderr(&without).contains("encrypted"), "{}", stderr(&without));
    assert!(!catalog.join(SIGNATURE).exists(), "refusal wrote a signature");

    let wrong = run_with(
        &[
            "catalog",
            "sign",
            path_text(&catalog),
            "--key",
            path_text(&key),
            "--password-env",
            variable,
        ],
        &[(variable, "wrong password")],
    );
    assert!(!wrong.status.success());
    assert!(
        stderr(&wrong).contains("cannot decrypt secret key"),
        "{}",
        stderr(&wrong)
    );
    assert!(!catalog.join(SIGNATURE).exists(), "refusal wrote a signature");

    let unset = run(&[
        "catalog",
        "sign",
        path_text(&catalog),
        "--key",
        path_text(&key),
        "--password-env",
        "MODN_TEST_CATALOG_KEY_PASSWORD_UNSET",
    ]);
    assert!(!unset.status.success());
    assert!(
        stderr(&unset).contains("MODN_TEST_CATALOG_KEY_PASSWORD_UNSET"),
        "{}",
        stderr(&unset)
    );
    assert!(!catalog.join(SIGNATURE).exists(), "refusal wrote a signature");

    let signed = run_with(
        &[
            "--json",
            "catalog",
            "sign",
            path_text(&catalog),
            "--key",
            path_text(&key),
            "--password-env",
            variable,
        ],
        &[(variable, "correct horse battery")],
    );
    assert!(signed.status.success(), "{}", stderr(&signed));
    let signature_bytes = fs::read(catalog.join(SIGNATURE)).expect("signature bytes");
    let verified = verify(&catalog, &catalog.join(PUBLIC_KEY));
    assert!(verified.status.success(), "{}", stderr(&verified));

    let later_refusal = run_with(
        &[
            "catalog",
            "sign",
            path_text(&catalog),
            "--key",
            path_text(&key),
            "--password-env",
            variable,
        ],
        &[(variable, "another password")],
    );
    assert!(!later_refusal.status.success());
    assert_eq!(
        fs::read(catalog.join(SIGNATURE)).expect("signature bytes"),
        signature_bytes
    );
}

#[test]
fn hash_prints_the_exact_blake3_pin_of_a_streamed_archive() {
    let temp = TempDir::new().expect("test fixture invariant");
    let archive = temp.path().join("example.zip");
    let bytes: Vec<u8> = (0..3u32 * 1024 * 1024)
        .map(|index| (index.wrapping_mul(31) % 251) as u8)
        .collect();
    fs::write(&archive, &bytes).expect("archive bytes");
    let expected = blake3::hash(&bytes).to_hex().to_string();
    assert_eq!(expected.len(), 64);
    assert_eq!(expected, expected.to_lowercase());

    let plain = run(&["catalog", "hash", path_text(&archive)]);
    assert!(plain.status.success(), "{}", stderr(&plain));
    assert_eq!(stdout(&plain), format!("{expected}\n"));

    let json_output = run(&["--json", "catalog", "hash", path_text(&archive)]);
    assert!(json_output.status.success(), "{}", stderr(&json_output));
    assert_eq!(stdout(&json_output), format!("{{\"blake3\":\"{expected}\"}}\n"));

    let state = temp.path().join("state");
    let missing = run(&[
        "--state-dir",
        path_text(&state),
        "catalog",
        "hash",
        path_text(&temp.path().join("absent.zip")),
    ]);
    assert!(!missing.status.success());
    assert!(
        stderr(&missing).contains("cannot read archive"),
        "{}",
        stderr(&missing)
    );
    assert!(!state.exists(), "hash wrote manager state");

    let directory = run(&["catalog", "hash", path_text(temp.path())]);
    assert!(!directory.status.success());
    assert!(
        stderr(&directory).contains("not a directory"),
        "{}",
        stderr(&directory)
    );
}

#[test]
fn json_verify_reports_the_catalog_name_and_digest() {
    let temp = TempDir::new().expect("test fixture invariant");
    let catalog = prepare_catalog(temp.path(), "catalog");
    let keys = temp.path().join("keys");
    assert!(keygen(&keys, "author").status.success());
    assert!(sign(&catalog, &keys.join("author.key")).status.success());
    let verified = json(&verify(&catalog, &catalog.join(PUBLIC_KEY)));
    assert_eq!(verified["catalog"], Value::from("community"));
    assert!(verified["digest"]
        .as_str()
        .expect("digest")
        .starts_with("blake3:"));
}
