use crate::buckets::{BucketStore, ZipCatalogSource};
use std::cell::Cell;
use std::fs;
use std::io::{self, Cursor, Read};
use std::path::Path;

fn source(path: &Path) -> ZipCatalogSource {
    ZipCatalogSource::new(path.to_str().expect("UTF-8 path").to_owned())
}

fn staging_left(state: &Path) -> bool {
    fs::read_dir(state.join("buckets"))
        .map(|mut entries| {
            entries.any(|entry| {
                entry
                    .map(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with("zip-catalog-")
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn assert_no_state(state: &Path) {
    assert!(!state.join("buckets/registrations/community.json").exists());
    assert!(!staging_left(state), "staging directory left behind");
}

#[test]
fn injected_download_failure_keeps_state_and_removes_staging() {
    let state = tempfile::tempdir().expect("state");
    let store = BucketStore::new(state.path());
    let zip = state.path().join("catalog.zip");
    let error = store
        .sync_zip_with_signer("community", Some(source(&zip)), |_| {
            anyhow::bail!("injected download failure")
        })
        .expect_err("download must fail");
    assert!(error.to_string().contains("injected download failure"));
    assert_no_state(state.path());
}

#[test]
fn truncated_download_is_refused_without_state() {
    let state = tempfile::tempdir().expect("state");
    let store = BucketStore::new(state.path());
    let zip = state.path().join("catalog.zip");
    let mut writer = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
    writer
        .start_file("bucket.json", zip::write::SimpleFileOptions::default())
        .expect("entry");
    std::io::Write::write_all(&mut writer, b"{}").expect("bytes");
    let bytes = writer.finish().expect("ZIP").into_inner();
    let truncated = bytes[..bytes.len() / 2].to_vec();
    let error = store
        .sync_zip_with_signer("community", Some(source(&zip)), move |_| {
            Ok(Box::new(Cursor::new(truncated)) as Box<dyn Read>)
        })
        .expect_err("truncated ZIP must fail");
    assert!(error.to_string().contains("invalid ZIP"), "{error}");
    assert_no_state(state.path());
}

#[test]
fn oversized_download_is_refused_without_state() {
    let state = tempfile::tempdir().expect("state");
    let store = BucketStore::new(state.path());
    let zip = state.path().join("catalog.zip");
    let error = store
        .sync_zip_with_signer("community", Some(source(&zip)), |_| {
            Ok(Box::new(io::repeat(0).take(super::MAX_CATALOG_ZIP_BYTES + 1)) as Box<dyn Read>)
        })
        .expect_err("oversized body must fail");
    assert!(error.to_string().contains("256 MiB size limit"), "{error}");
    assert_no_state(state.path());
}

#[test]
fn plain_http_is_refused_before_the_source_is_opened() {
    let state = tempfile::tempdir().expect("state");
    let store = BucketStore::new(state.path());
    let opened = Cell::new(false);
    let error = store
        .sync_zip_with_signer(
            "community",
            Some(ZipCatalogSource::new(
                "http://example.org/catalog.zip".to_owned(),
            )),
            |_| {
                opened.set(true);
                anyhow::bail!("opener must not run")
            },
        )
        .expect_err("plain HTTP must be refused");
    assert!(error.to_string().contains("must use https://"), "{error}");
    assert!(!opened.get(), "the opener ran for a refused source");
    assert_no_state(state.path());
}
