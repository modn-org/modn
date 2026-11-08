use super::{
    catalog_statement, main_bucket_public_key, verify_catalog, verify_catalog_directory,
    CatalogPublicKey, CatalogStatement,
};
use crate::bucket_digest::{catalog_digest, directory_digest, reject_case_collisions};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use minisign::KeyPair;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

const BUCKET_JSON: &str = r#"{"schema_version":1,"name":"test","title":"Test"}"#;
const MOD_JSON: &str = r#"{"schema_version":1,"id":"mod"}"#;
const PINNED_BLAKE3: &str =
    "blake3:29576b11ea5785cce9d64eb2a56ac07a2b8ddabed0c3c05a3bebf031f623b33e";
const PINNED_SHA256: &str = "6da1fff4583f24044eccfec3d3d02d8c732924bc3aea11ed63bfb3bce56661f1";

fn catalog(name: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir_all(directory.path().join("games/test-game")).unwrap();
    fs::write(
        directory.path().join("bucket.json"),
        format!(
            r#"{{"schema_version":1,"name":{},"title":"Test"}}"#,
            serde_json::to_string(name).unwrap()
        ),
    )
    .unwrap();
    fs::write(directory.path().join("games/test-game/mod.json"), MOD_JSON).unwrap();
    directory
}

fn fixed_catalog() -> tempfile::TempDir {
    catalog("test")
}

struct TestKey {
    pair: KeyPair,
    public: CatalogPublicKey,
    bare: String,
    two_line: String,
}

fn test_key() -> TestKey {
    let pair = KeyPair::generate_unencrypted_keypair().unwrap();
    let two_line = pair.pk.to_box().unwrap().into_string();
    let bare = two_line.lines().nth(1).unwrap().to_owned();
    let public = CatalogPublicKey::parse(&two_line).unwrap();
    TestKey {
        pair,
        public,
        bare,
        two_line,
    }
}

fn sign(key: &TestKey, statement: &CatalogStatement) -> String {
    minisign::sign(
        Some(&key.pair.pk),
        &key.pair.sk,
        Cursor::new(statement.as_bytes()),
        Some("catalog test"),
        Some("catalog test signature"),
    )
    .unwrap()
    .into_string()
}

#[test]
fn catalog_digest_matches_pinned_framing() {
    let directory = fixed_catalog();
    let digest = catalog_digest(directory.path()).unwrap();
    assert_eq!(digest.as_str(), PINNED_BLAKE3);

    let mut framed = Vec::new();
    framed.extend_from_slice(b"modn-catalog-blake3/v1");
    framed.push(0);
    for (path, contents) in [
        ("bucket.json", BUCKET_JSON),
        ("games/test-game/mod.json", MOD_JSON),
    ] {
        framed.extend_from_slice(path.as_bytes());
        framed.push(0);
        framed.extend_from_slice(&(contents.len() as u64).to_le_bytes());
        framed.extend_from_slice(contents.as_bytes());
    }
    assert_eq!(
        format!("blake3:{}", blake3::hash(&framed).to_hex()),
        PINNED_BLAKE3
    );
}

#[test]
fn statement_has_exact_bytes() {
    let directory = fixed_catalog();
    let statement = catalog_statement(directory.path()).unwrap();
    let expected = concat!(
        "modn-catalog-signature/v1\n",
        "catalog: test\n",
        "digest: blake3:29576b11ea5785cce9d64eb2a56ac07a2b8ddabed0c3c05a3bebf031f623b33e\n"
    );
    assert_eq!(statement.as_bytes(), expected.as_bytes());
    assert_eq!(statement.name(), "test");
    assert_eq!(statement.digest().as_str(), PINNED_BLAKE3);
}

#[test]
fn existing_sha256_digest_is_unchanged() {
    let directory = fixed_catalog();
    assert_eq!(directory_digest(directory.path()).unwrap(), PINNED_SHA256);
}

#[test]
fn root_signature_files_are_excluded_but_nested_files_are_included() {
    let directory = fixed_catalog();
    let baseline = catalog_digest(directory.path()).unwrap();
    fs::write(directory.path().join("catalog.sig"), "signature").unwrap();
    fs::write(directory.path().join("catalog.pub"), "key").unwrap();
    assert_eq!(catalog_digest(directory.path()).unwrap(), baseline);

    fs::create_dir_all(directory.path().join("nested")).unwrap();
    fs::write(directory.path().join("nested/catalog.sig"), "signature").unwrap();
    assert_ne!(catalog_digest(directory.path()).unwrap(), baseline);
    fs::remove_file(directory.path().join("nested/catalog.sig")).unwrap();
    fs::write(directory.path().join("nested/catalog.pub"), "key").unwrap();
    assert_ne!(catalog_digest(directory.path()).unwrap(), baseline);
}

#[test]
fn root_git_entry_is_refused() {
    let directory = fixed_catalog();
    fs::create_dir(directory.path().join(".git")).unwrap();
    let error = catalog_digest(directory.path()).unwrap_err();
    assert!(error.to_string().contains(".git"));
}

fn symlink_file(original: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(original, link)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(original, link)
    }
}

#[test]
fn symlink_entry_is_refused() {
    let directory = fixed_catalog();
    let original = directory.path().join("bucket.json");
    let link = directory.path().join("linked.json");
    if let Err(error) = symlink_file(&original, &link) {
        #[cfg(windows)]
        if matches!(error.raw_os_error(), Some(5) | Some(1314)) {
            eprintln!("skipped: cannot create a symlink without Developer Mode: {error}");
            return;
        }
        panic!("cannot create symlink fixture: {error}");
    }
    let error = catalog_digest(directory.path()).unwrap_err();
    assert!(error.to_string().contains("non-regular catalog entry"));
}

#[cfg(windows)]
fn non_utf8_path(directory: &Path) -> std::io::Result<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    let path = directory.join(OsString::from_wide(&[0xD800]));
    fs::write(&path, b"x")?;
    Ok(path)
}

#[cfg(unix)]
fn non_utf8_path(directory: &Path) -> std::io::Result<PathBuf> {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let path = directory.join(OsString::from_vec(vec![0xFF]));
    fs::write(&path, b"x")?;
    Ok(path)
}

#[test]
fn non_utf8_path_is_refused() {
    let directory = fixed_catalog();
    if non_utf8_path(directory.path()).is_err() {
        eprintln!("skipped: this platform cannot create a non-UTF-8 file name");
        return;
    }
    let error = catalog_digest(directory.path()).unwrap_err();
    assert!(error.to_string().contains("non-UTF-8 catalog path"));
}

#[test]
fn case_insensitive_path_collision_is_refused() {
    let entries = vec![
        ("Games/Mod.json".to_owned(), PathBuf::new()),
        ("games/mod.json".to_owned(), PathBuf::new()),
    ];
    let error = reject_case_collisions(&entries).unwrap_err();
    assert!(error.to_string().contains("case-insensitively"));
}

#[cfg(unix)]
#[test]
fn case_insensitive_tree_collision_is_refused() {
    let directory = fixed_catalog();
    fs::write(directory.path().join("Games.json"), b"a").unwrap();
    fs::write(directory.path().join("games.json"), b"b").unwrap();
    let error = catalog_digest(directory.path()).unwrap_err();
    assert!(error.to_string().contains("case-insensitively"));
}

#[test]
fn catalog_name_with_newline_is_rejected() {
    let directory = catalog("bad\nname");
    let error = catalog_statement(directory.path()).unwrap_err();
    assert!(error.to_string().contains("catalog name"));
}

#[test]
fn both_public_key_formats_parse_with_sixteen_digit_fingerprint() {
    let key = test_key();
    let from_file = CatalogPublicKey::parse(&key.two_line).unwrap();
    let from_bare = CatalogPublicKey::parse(&key.bare).unwrap();
    assert_eq!(from_file.key_id(), from_bare.key_id());
    let fingerprint = from_bare.key_id().to_string();
    assert_eq!(fingerprint.len(), 16);
    assert!(fingerprint
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte)));
    assert!(key.two_line.contains(&fingerprint));
}

#[cfg(not(feature = "test-main-key"))]
#[test]
fn main_bucket_key_uses_embedded_text_and_ignores_override() {
    let embedded = test_key();
    let foreign = test_key();
    std::env::set_var("MODN_TEST_MAIN_BUCKET_PUBLIC_KEY", &foreign.bare);
    let resolved = main_bucket_public_key(&embedded.two_line).unwrap();
    std::env::remove_var("MODN_TEST_MAIN_BUCKET_PUBLIC_KEY");
    assert_eq!(resolved.key_id(), embedded.public.key_id());
}

#[cfg(feature = "test-main-key")]
#[test]
fn main_bucket_key_prefers_override_and_refuses_placeholder() {
    let placeholder =
        "UNSET: replace this file with the minisign public key of the main bucket signer";
    let embedded = test_key();
    let foreign = test_key();

    std::env::set_var("MODN_TEST_MAIN_BUCKET_PUBLIC_KEY", &foreign.bare);
    let overridden = main_bucket_public_key(placeholder).unwrap();
    assert_eq!(overridden.key_id(), foreign.public.key_id());
    std::env::remove_var("MODN_TEST_MAIN_BUCKET_PUBLIC_KEY");

    let error = main_bucket_public_key(placeholder).unwrap_err();
    assert!(error
        .to_string()
        .contains("This Modn build has no main bucket signer key."));

    let resolved = main_bucket_public_key(&embedded.two_line).unwrap();
    assert_eq!(resolved.key_id(), embedded.public.key_id());
}

#[test]
fn malformed_public_key_is_refused() {
    for text in ["", "not a minisign key", "RWnotbase64"] {
        let error = CatalogPublicKey::parse(text).unwrap_err();
        assert!(
            error.to_string().contains("malformed catalog public key"),
            "{text:?} produced {error}"
        );
    }
}

#[test]
fn valid_signature_verifies() {
    let directory = fixed_catalog();
    let statement = catalog_statement(directory.path()).unwrap();
    let key = test_key();
    let signature = sign(&key, &statement);
    let verified = verify_catalog(directory.path(), &key.public, &signature).unwrap();
    assert_eq!(verified.statement, statement);
    assert_eq!(verified.key_id, key.public.key_id());
}

#[test]
fn catalog_signature_file_is_used_and_missing_file_is_refused() {
    let directory = fixed_catalog();
    let statement = catalog_statement(directory.path()).unwrap();
    let key = test_key();
    let signature = sign(&key, &statement);
    fs::write(directory.path().join("catalog.sig"), &signature).unwrap();
    let verified = verify_catalog_directory(directory.path(), &key.public).unwrap();
    assert_eq!(verified.statement, statement);

    fs::remove_file(directory.path().join("catalog.sig")).unwrap();
    let error = verify_catalog_directory(directory.path(), &key.public).unwrap_err();
    assert!(error.to_string().contains("catalog signature is missing"));
}

#[test]
fn malformed_signature_is_refused() {
    let directory = fixed_catalog();
    let key = test_key();
    for text in [
        "",
        "not a signature",
        "untrusted comment: x\nnot base64\ntrusted comment: x\nbogus",
    ] {
        let error = verify_catalog(directory.path(), &key.public, text).unwrap_err();
        assert!(
            error.to_string().contains("malformed catalog signature"),
            "{text:?} produced {error}"
        );
    }
}

#[test]
fn wrong_key_is_refused_and_names_both_fingerprints() {
    let directory = fixed_catalog();
    let statement = catalog_statement(directory.path()).unwrap();
    let signer = test_key();
    let verifier = test_key();
    let signature = sign(&signer, &statement);
    let error = verify_catalog(directory.path(), &verifier.public, &signature)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(&signer.public.key_id().to_string()),
        "{error}"
    );
    assert!(
        error.contains(&verifier.public.key_id().to_string()),
        "{error}"
    );
    assert!(error.contains("does not match public key"), "{error}");
}

#[test]
fn signature_over_modified_tree_is_refused() {
    let directory = fixed_catalog();
    let statement = catalog_statement(directory.path()).unwrap();
    let key = test_key();
    let signature = sign(&key, &statement);
    fs::write(
        directory.path().join("games/test-game/mod.json"),
        r#"{"schema_version":1,"id":"changed"}"#,
    )
    .unwrap();
    let error = verify_catalog(directory.path(), &key.public, &signature).unwrap_err();
    assert!(error
        .to_string()
        .contains("catalog signature verification failed"));
}

#[test]
fn legacy_signature_is_refused() {
    let directory = fixed_catalog();
    let statement = catalog_statement(directory.path()).unwrap();
    let key = test_key();
    let signature = sign(&key, &statement);
    let mut lines: Vec<String> = signature.lines().map(str::to_owned).collect();
    let mut bytes = STANDARD.decode(lines[1].trim()).unwrap();
    bytes[0..2].copy_from_slice(b"Ed");
    lines[1] = STANDARD.encode(bytes);
    let legacy = format!("{}\n", lines.join("\n"));

    let error = verify_catalog(directory.path(), &key.public, &legacy).unwrap_err();
    assert!(error.to_string().contains("legacy catalog signatures"));
}
