#[path = "support/catalog_author.rs"]
mod catalog_author;
#[path = "support/catalog_authoring.rs"]
mod catalog_authoring;
#[path = "support/main_bucket.rs"]
mod main_bucket;

use catalog_author::{keygen, sign, verify};
use catalog_authoring::{json, path_text, prepare_catalog, run, run_with, stderr, stdout};
use serde_json::Value;
use std::fs;
use tempfile::TempDir;

const SIGNATURE: &str = "catalog.sig";
const PUBLIC_KEY: &str = "catalog.pub";

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
    assert!(
        fs::read_to_string(&key)
            .expect("secret key")
            .lines()
            .count()
            >= 2
    );

    let without = run(&[
        "catalog",
        "sign",
        path_text(&catalog),
        "--key",
        path_text(&key),
    ]);
    assert!(!without.status.success());
    assert!(
        stderr(&without).contains("encrypted"),
        "{}",
        stderr(&without)
    );
    assert!(
        !catalog.join(SIGNATURE).exists(),
        "refusal wrote a signature"
    );

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
    assert!(
        !catalog.join(SIGNATURE).exists(),
        "refusal wrote a signature"
    );

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
    assert!(
        !catalog.join(SIGNATURE).exists(),
        "refusal wrote a signature"
    );

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
    assert_eq!(
        stdout(&json_output),
        format!("{{\"blake3\":\"{expected}\"}}\n")
    );

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
