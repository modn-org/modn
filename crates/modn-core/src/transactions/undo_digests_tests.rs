use super::catalog_tests::{plan, Fixture};
use crate::history::HistoryFile;
use crate::progress::{CancellationToken, OperationControl};
use std::fs::File;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;

fn cancelled_control() -> OperationControl {
    let token = CancellationToken::new();
    token.cancel();
    OperationControl::new(token, |_| {})
}

fn expected(destination: &str, contents: &[u8]) -> Vec<HistoryFile> {
    vec![HistoryFile {
        root: "game".into(),
        destination: destination.into(),
        digest_before: Some(blake3::hash(contents).to_hex().to_string()),
        digest_after: None,
    }]
}

fn zip_archive(directory: &Path, contents: &[u8]) -> PathBuf {
    let path = directory.join("preflight.zip");
    let mut zip = zip::ZipWriter::new(File::create(&path).expect("archive"));
    zip.start_file("pack/mod.plugin", SimpleFileOptions::default())
        .expect("entry");
    zip.write_all(contents).expect("payload");
    zip.finish().expect("finish");
    path
}

fn sevenz_archive(directory: &Path, contents: &[u8]) -> PathBuf {
    let path = directory.join("preflight.7z");
    let mut writer = sevenz_rust2::ArchiveWriter::create(&path).expect("7z writer");
    writer
        .push_archive_entry(
            sevenz_rust2::ArchiveEntry::new_file("pack/mod.plugin"),
            Some(Cursor::new(contents)),
        )
        .expect("7z member");
    writer.finish().expect("complete 7z");
    path
}

#[test]
fn cancelled_zip_digest_read_returns_cancelled() {
    let mut fixture = Fixture::new();
    let directory = fixture.game.parent().expect("fixture directory");
    let archive = zip_archive(directory, b"zip-bytes");
    let plan = plan(&fixture.game, &archive);
    fixture.store.set_control(cancelled_control());

    let error = fixture
        .store
        .verify_restored_bytes(
            &archive,
            &plan,
            &expected("addons/mod.plugin", b"zip-bytes"),
        )
        .expect_err("cancelled read");
    assert!(
        error.downcast_ref::<crate::progress::Cancelled>().is_some(),
        "{error:#}"
    );
}

#[test]
fn cancelled_7z_digest_read_returns_cancelled() {
    let mut fixture = Fixture::new();
    let directory = fixture.game.parent().expect("fixture directory");
    let archive = sevenz_archive(directory, b"7z-bytes");
    let plan = plan(&fixture.game, &archive);
    fixture.store.set_control(cancelled_control());

    let error = fixture
        .store
        .verify_restored_bytes(&archive, &plan, &expected("addons/mod.plugin", b"7z-bytes"))
        .expect_err("cancelled read");
    assert!(
        error.downcast_ref::<crate::progress::Cancelled>().is_some(),
        "{error:#}"
    );
}
