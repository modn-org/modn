#[path = "support/main_bucket.rs"]
mod main_bucket;
#[path = "support/zip_catalog.rs"]
mod zip_catalog;
#[path = "support/zip_catalog_fixture.rs"]
mod zip_catalog_fixture;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Output;
use zip_catalog_fixture::{assert_refused_without_state, Fixture};
fn zip_with_entries(path: &Path, entries: &[(&str, &[u8])]) {
    let mut writer = zip::ZipWriter::new(fs::File::create(path).expect("hostile ZIP"));
    for (name, bytes) in entries {
        writer
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .expect("hostile entry");
        writer.write_all(bytes).expect("hostile bytes");
    }
    writer.finish().expect("hostile ZIP complete");
}
fn raw_zip(path: &Path, entries: &[(&str, &[u8])]) {
    fn put(bytes: &mut Vec<u8>, value: &[u8]) {
        bytes.extend_from_slice(value);
    }
    let mut body = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let offset = body.len() as u32;
        let size = data.len() as u32;
        let mut header = Vec::new();
        put(&mut header, &0x0403_4b50u32.to_le_bytes());
        put(&mut header, &20u16.to_le_bytes());
        put(&mut header, &0u16.to_le_bytes());
        put(&mut header, &0u16.to_le_bytes());
        put(&mut header, &0u16.to_le_bytes());
        put(&mut header, &0u16.to_le_bytes());
        put(&mut header, &0u32.to_le_bytes());
        put(&mut header, &size.to_le_bytes());
        put(&mut header, &size.to_le_bytes());
        put(&mut header, &(name.len() as u16).to_le_bytes());
        put(&mut header, &0u16.to_le_bytes());
        put(&mut header, name.as_bytes());
        body.extend_from_slice(&header);
        body.extend_from_slice(data);
        put(&mut central, &0x0201_4b50u32.to_le_bytes());
        put(&mut central, &20u16.to_le_bytes());
        put(&mut central, &20u16.to_le_bytes());
        put(&mut central, &0u16.to_le_bytes());
        put(&mut central, &0u16.to_le_bytes());
        put(&mut central, &0u16.to_le_bytes());
        put(&mut central, &0u16.to_le_bytes());
        put(&mut central, &0u32.to_le_bytes());
        put(&mut central, &size.to_le_bytes());
        put(&mut central, &size.to_le_bytes());
        put(&mut central, &(name.len() as u16).to_le_bytes());
        put(&mut central, &0u16.to_le_bytes());
        put(&mut central, &0u16.to_le_bytes());
        put(&mut central, &0u16.to_le_bytes());
        put(&mut central, &0u16.to_le_bytes());
        put(&mut central, &0u32.to_le_bytes());
        put(&mut central, &offset.to_le_bytes());
        put(&mut central, name.as_bytes());
    }
    let central_offset = body.len() as u32;
    body.extend_from_slice(&central);
    put(&mut body, &0x0605_4b50u32.to_le_bytes());
    put(&mut body, &0u16.to_le_bytes());
    put(&mut body, &0u16.to_le_bytes());
    put(&mut body, &(entries.len() as u16).to_le_bytes());
    put(&mut body, &(entries.len() as u16).to_le_bytes());
    put(&mut body, &(central.len() as u32).to_le_bytes());
    put(&mut body, &central_offset.to_le_bytes());
    put(&mut body, &0u16.to_le_bytes());
    fs::write(path, body).expect("raw ZIP");
}
fn hostile(fixture: &Fixture, name: &str, build: impl FnOnce(&Path)) -> Output {
    let zip = fixture.temp.path().join(format!("{name}.zip"));
    build(&zip);
    fixture.sync(&zip)
}
#[test]
fn hostile_zip_entries_are_refused() {
    let fixture = Fixture::new();
    for (name, entry, expected) in [
        ("traversal", "../escape", "unsafe archive entry"),
        ("absolute", "/escape", "unsafe archive entry"),
        ("backslash", "mods\\escape", "backslash separator"),
    ] {
        let output = hostile(&fixture, name, |zip| {
            zip_with_entries(zip, &[(entry, b"x")])
        });
        assert_refused_without_state(&fixture, &output, expected);
        assert!(
            !fixture
                .bucket_entries()
                .iter()
                .any(|name| name.contains("escape")),
            "entry escaped staging: {:?}",
            fixture.bucket_entries()
        );
    }
    let output = hostile(&fixture, "duplicate", |zip| {
        raw_zip(zip, &[("bucket.json", b"{}"), ("bucket.json", b"{}")])
    });
    assert_refused_without_state(&fixture, &output, "duplicate");
    let output = hostile(&fixture, "case", |zip| {
        zip_with_entries(zip, &[("Mods/a", b"x"), ("mods/a", b"y")])
    });
    assert_refused_without_state(&fixture, &output, "case-colliding");
}
#[test]
fn hostile_zip_symlink_and_limits_are_refused() {
    let fixture = Fixture::new();
    let zip = fixture.temp.path().join("symlink.zip");
    let mut writer = zip::ZipWriter::new(fs::File::create(&zip).expect("symlink ZIP"));
    writer
        .add_symlink("link", "target", zip::write::SimpleFileOptions::default())
        .expect("symlink entry");
    writer.finish().expect("symlink ZIP");
    let mut stored =
        zip::ZipArchive::new(fs::File::open(&zip).expect("symlink ZIP")).expect("open symlink ZIP");
    assert!(
        stored.by_name("link").expect("stored link").is_symlink(),
        "the fixture entry is a regular file, not a symlink"
    );
    drop(stored);
    let output = fixture.sync(&zip);
    assert_refused_without_state(&fixture, &output, "symlink");
    let output = hostile(&fixture, "ratio", |zip| {
        let mut writer = zip::ZipWriter::new(fs::File::create(zip).expect("ratio ZIP"));
        writer
            .start_file("zeros", zip::write::SimpleFileOptions::default())
            .expect("ratio entry");
        writer.write_all(&vec![0u8; 1024 * 1024]).expect("zeros");
        writer.finish().expect("ratio ZIP");
    });
    assert_refused_without_state(&fixture, &output, "expansion ratio");
    let output = hostile(&fixture, "oversized", |zip| {
        let mut writer = zip::ZipWriter::new(fs::File::create(zip).expect("oversized ZIP"));
        writer
            .start_file("big", zip::write::SimpleFileOptions::default())
            .expect("oversized entry");
        for _ in 0..65 {
            writer
                .write_all(&vec![0u8; 1024 * 1024])
                .expect("big bytes");
        }
        writer.finish().expect("oversized ZIP");
    });
    assert_refused_without_state(&fixture, &output, "64 MiB size limit");
}
#[test]
fn hostile_zip_entry_count_is_refused() {
    let fixture = Fixture::new();
    let output = hostile(&fixture, "many", |zip| {
        let mut writer = zip::ZipWriter::new(fs::File::create(zip).expect("many ZIP"));
        for index in 0..20_001 {
            writer
                .start_file(
                    format!("entries/{index}.txt"),
                    zip::write::SimpleFileOptions::default(),
                )
                .expect("many entry");
        }
        writer.finish().expect("many ZIP");
    });
    assert_refused_without_state(&fixture, &output, "more than 20000 entries");
}
