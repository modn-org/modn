//! The official `main` sequence record survives a failed swap.

use crate::buckets::BucketStore;
use crate::error::{code_of, ErrorCode};
use crate::manager_roots::ManagerRoots;
use minisign::KeyPair;
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Official bucket roots whose next swap fails after the sequence is recorded.
static FAIL_NEXT_SWAP: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

/// Fail once for `root` when a test armed it. The set is keyed by root, so
/// parallel tests never see each other's failure.
pub(super) fn fail_point(root: &Path) -> anyhow::Result<()> {
    let armed = FAIL_NEXT_SWAP.lock().expect("fail point lock").remove(root);
    if armed {
        anyhow::bail!("injected main bucket swap failure");
    }
    Ok(())
}

fn tree(parent: &Path, sequence: u64) -> PathBuf {
    let tree = parent.join(format!("tree-{sequence}"));
    fs::create_dir_all(&tree).expect("fixture tree");
    fs::write(
        tree.join("bucket.json"),
        r#"{"schema_version":2,"name":"main","title":"Main"}"#,
    )
    .expect("fixture metadata");
    fs::write(
        tree.join("sequence.json"),
        format!(r#"{{"schema_version":1,"sequence":{sequence}}}"#),
    )
    .expect("fixture sequence");
    tree
}

fn pack(tree: &Path, zip: &Path) {
    let mut writer = zip::ZipWriter::new(fs::File::create(zip).expect("fixture ZIP"));
    for name in ["bucket.json", "sequence.json"] {
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .expect("fixture ZIP entry");
        writer
            .write_all(&fs::read(tree.join(name)).expect("fixture file"))
            .expect("fixture ZIP bytes");
    }
    writer.finish().expect("fixture ZIP complete");
}

fn update(store: &BucketStore, tree: &Path) -> anyhow::Result<()> {
    let zip = tree.with_extension("zip");
    pack(tree, &zip);
    let key = KeyPair::generate_unencrypted_keypair().expect("fixture key");
    let public = key.pk.to_box().expect("fixture key box").into_string();
    let statement = crate::buckets::catalog_statement(tree).expect("fixture statement");
    let signature = minisign::sign(
        Some(&key.pk),
        &key.sk,
        Cursor::new(statement.as_bytes()),
        Some("sequence fixture"),
        Some("sequence fixture signature"),
    )
    .expect("fixture signature")
    .into_string();
    store
        .update_main(
            &public,
            || -> anyhow::Result<fs::File> { Ok(fs::File::open(&zip)?) },
            || -> anyhow::Result<io::Cursor<Vec<u8>>> {
                Ok(io::Cursor::new(signature.clone().into_bytes()))
            },
        )
        .map(|_| ())
}

fn recorded(store: &BucketStore) -> Option<u64> {
    store
        .main_sequence()
        .expect("main sequence record")
        .map(|accepted| accepted.sequence)
}

fn installed_sequence(roots: &ManagerRoots) -> String {
    fs::read_to_string(roots.official_main().join("sequence.json")).expect("installed sequence")
}

#[test]
fn a_failed_swap_keeps_the_new_sequence_and_refuses_an_older_catalog() {
    let state = tempfile::tempdir().expect("state");
    let fixtures = tempfile::tempdir().expect("fixtures");
    let roots = ManagerRoots::new(state.path(), None);
    let store = BucketStore::new(&roots);
    update(&store, &tree(fixtures.path(), 2)).expect("first numbered update");
    assert_eq!(recorded(&store), Some(2));

    FAIL_NEXT_SWAP
        .lock()
        .expect("fail point lock")
        .insert(roots.official_bucket_root());
    let error = update(&store, &tree(fixtures.path(), 3)).expect_err("injected failure");
    assert!(
        format!("{error:#}").contains("injected main bucket swap failure"),
        "{error:#}"
    );
    assert_eq!(recorded(&store), Some(3), "the record leads the swap");
    assert_eq!(
        installed_sequence(&roots),
        r#"{"schema_version":1,"sequence":2}"#,
        "the previous main copy stays installed"
    );

    let older = tempfile::tempdir().expect("older fixtures");
    let error = update(&store, &tree(older.path(), 2)).expect_err("rollback");
    assert_eq!(
        code_of(&error),
        Some(ErrorCode::CatalogRollback),
        "{error:#}"
    );
    assert_eq!(recorded(&store), Some(3));

    update(&store, &tree(fixtures.path(), 3)).expect("retry of the same catalog");
    assert_eq!(recorded(&store), Some(3));
    assert_eq!(
        installed_sequence(&roots),
        r#"{"schema_version":1,"sequence":3}"#
    );
}
