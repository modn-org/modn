use super::super::{ArchiveCache, CacheKey, CacheUse};
use super::{index_path, ArchiveIndex, ArchivePin};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

const ALIAS_URL: &str = "https://alias.invalid/alias.zip";
const BYTES: &[u8] = b"shared verified bytes";

fn pin(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn alias_key(digest: &str) -> CacheKey<'_> {
    CacheKey::new("alias-game", "alias", "9", ALIAS_URL, digest)
}

/// Store `body` under a fresh package release, downloading it every time so
/// two keys can hold the same digest without one reusing the other's path.
fn store(cache: &ArchiveCache, key: &CacheKey<'_>, body: &[u8]) -> PathBuf {
    cache
        .acquire(key, CacheUse::Refresh, |_| Ok(Cursor::new(body.to_vec())))
        .unwrap()
        .path
}

fn name_of(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

fn record(root: &Path, pin: &ArchivePin, name: &str) {
    let mut index = ArchiveIndex::read(root).unwrap_or_default();
    index.insert(pin.clone(), name.to_owned());
    index.write(root).unwrap();
}

/// Every file name in `root`, so a lookup that writes any file is visible.
fn file_names(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// An entry whose name sorts before `zzz-game`, so a scan finds it first.
fn first_entry(cache: &ArchiveCache, digest: &str) -> PathBuf {
    store(
        cache,
        &CacheKey::new("aaa-game", "mod", "1", "https://aaa.invalid/a.zip", digest),
        BYTES,
    )
}

/// A second, later-sorting entry for the same digest.
fn second_entry(cache: &ArchiveCache, digest: &str) -> PathBuf {
    store(
        cache,
        &CacheKey::new("zzz-game", "mod", "1", "https://zzz.invalid/z.zip", digest),
        BYTES,
    )
}

#[test]
fn pin_codec_names_one_algorithm() {
    let digest = pin(BYTES);
    let pin = ArchivePin::blake3(&digest).unwrap();
    assert_eq!(pin.encode(), format!("blake3:{digest}"));
    assert_eq!(ArchivePin::decode(&pin.encode()), Some(pin));
    assert!(ArchivePin::blake3(&digest.to_uppercase()).is_none());
    assert!(ArchivePin::blake3("abc").is_none());
    assert!(ArchivePin::decode(&format!("sha256:{digest}")).is_none());
}

#[test]
fn lookup_uses_the_index_and_matches_the_scan_result() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let digest = pin(BYTES);
    let first = first_entry(&cache, &digest);
    let second = second_entry(&cache, &digest);
    assert!(first < second, "listing order must prefer the first entry");

    let pin = ArchivePin::blake3(&digest).unwrap();
    record(cache.root(), &pin, &name_of(&second));
    assert_eq!(
        cache.cached(&alias_key(&digest)).unwrap(),
        Some(second.clone()),
        "the index decides which verified entry is returned"
    );

    fs::remove_file(index_path(cache.root())).unwrap();
    let before = file_names(cache.root());
    assert_eq!(
        cache.cached(&alias_key(&digest)).unwrap(),
        Some(first.clone()),
        "a scan returns the first verified entry in listing order"
    );
    assert_eq!(
        file_names(cache.root()),
        before,
        "a lookup must not write the index it scanned"
    );
}

#[test]
fn a_missing_index_falls_back_to_a_scan_and_is_not_written() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let digest = pin(BYTES);
    let entry = first_entry(&cache, &digest);
    fs::remove_file(index_path(cache.root())).unwrap();

    let before = file_names(cache.root());
    assert_eq!(
        cache.cached(&alias_key(&digest)).unwrap(),
        Some(entry.clone())
    );
    assert_eq!(
        file_names(cache.root()),
        before,
        "a missing index stays missing after a read-only lookup"
    );
}

#[test]
fn a_corrupt_index_falls_back_to_a_scan_without_being_rewritten() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let digest = pin(BYTES);
    let entry = first_entry(&cache, &digest);
    let corrupt: [&[u8]; 4] = [
        b"garbage bytes",
        br#"{"version":1,"entries":{"sha256:"#,
        br#"{"version":2,"entries":{}}"#,
        br#"{"version":1,"entries":{},"future":true}"#,
    ];

    for payload in corrupt {
        fs::write(index_path(cache.root()), payload).unwrap();
        let before = file_names(cache.root());
        assert_eq!(
            cache.cached(&alias_key(&digest)).unwrap(),
            Some(entry.clone()),
            "corrupt index {:?} must not hide the entry",
            String::from_utf8_lossy(payload)
        );
        assert_eq!(
            fs::read(index_path(cache.root())).unwrap(),
            payload,
            "a read-only lookup must not rewrite a corrupt index"
        );
        assert_eq!(file_names(cache.root()), before);
    }
}

#[test]
fn stale_index_entries_are_never_returned_or_rewritten() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let digest = pin(BYTES);
    let first = first_entry(&cache, &digest);
    let second = second_entry(&cache, &digest);
    let pin = ArchivePin::blake3(&digest).unwrap();

    record(cache.root(), &pin, &name_of(&second));
    fs::remove_file(&second).unwrap();
    assert_eq!(
        cache.cached(&alias_key(&digest)).unwrap(),
        Some(first.clone())
    );
    let unchanged = ArchiveIndex::read(cache.root()).unwrap();
    assert_eq!(
        unchanged.file_name(&pin),
        Some(name_of(&second).as_str()),
        "a stale record stays until a writing path replaces the index"
    );

    fs::write(&second, b"tampered").unwrap();
    record(cache.root(), &pin, &name_of(&second));
    assert_eq!(cache.cached(&alias_key(&digest)).unwrap(), Some(first));
    let unchanged = ArchiveIndex::read(cache.root()).unwrap();
    assert_eq!(unchanged.file_name(&pin), Some(name_of(&second).as_str()));
}

#[test]
fn a_tampered_indexed_entry_is_never_returned() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let digest = pin(BYTES);
    let entry = first_entry(&cache, &digest);
    let pin = ArchivePin::blake3(&digest).unwrap();
    fs::write(&entry, b"tampered").unwrap();

    assert_eq!(cache.cached(&alias_key(&digest)).unwrap(), None);
    let unchanged = ArchiveIndex::read(cache.root()).unwrap();
    assert_eq!(
        unchanged.file_name(&pin),
        Some(name_of(&entry).as_str()),
        "the stale pin is not returned and is not rewritten by a lookup"
    );
}

#[test]
fn acquire_records_the_stored_entry_in_the_index() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let digest = pin(BYTES);
    let entry = first_entry(&cache, &digest);
    let name = name_of(&entry);
    let text = fs::read_to_string(index_path(cache.root())).unwrap();
    assert!(
        text.contains(&format!("\"blake3:{digest}\":\"{name}\"")),
        "{text}"
    );

    let entries = cache.entries(None).unwrap();
    assert_eq!(entries.len(), 1, "the index file is not a cache entry");
    assert_eq!(entries[0].path, entry);

    fs::write(
        cache.root().join(".digest-index-interrupted.tmp"),
        b"partial",
    )
    .unwrap();
    assert_eq!(
        cache.entries(None).unwrap().len(),
        1,
        "an index temporary file is not a cache entry"
    );
    assert_eq!(cache.remove(None).unwrap().len(), 1);
    assert!(
        index_path(cache.root()).is_file(),
        "removing entries leaves the index; a stale pin falls back to a scan"
    );
    assert_eq!(cache.cached(&alias_key(&digest)).unwrap(), None);
}

#[test]
fn a_legacy_sha256_index_is_ignored_without_being_rewritten() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let digest = pin(BYTES);
    let entry = first_entry(&cache, &digest);
    let name = name_of(&entry);
    let legacy = format!(r#"{{"version":1,"entries":{{"sha256:{digest}":"{name}"}}}}"#);
    fs::write(index_path(cache.root()), &legacy).unwrap();

    assert_eq!(
        cache.cached(&alias_key(&digest)).unwrap(),
        Some(entry.clone()),
        "a legacy index must not hide a verified entry"
    );
    assert_eq!(
        fs::read_to_string(index_path(cache.root())).unwrap(),
        legacy,
        "a read-only lookup must not migrate a legacy index"
    );
}

#[test]
fn a_read_only_index_load_writes_nothing_for_a_missing_or_legacy_index() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let digest = pin(BYTES);
    let entry = first_entry(&cache, &digest);
    let name = name_of(&entry);

    fs::remove_file(index_path(cache.root())).unwrap();
    let before = file_names(cache.root());
    assert_eq!(
        cache.cached(&alias_key(&digest)).unwrap(),
        Some(entry.clone())
    );
    assert_eq!(
        file_names(cache.root()),
        before,
        "a missing index must stay missing"
    );

    let legacy = format!(r#"{{"version":1,"entries":{{"sha256:{digest}":"{name}"}}}}"#);
    fs::write(index_path(cache.root()), &legacy).unwrap();
    let before = file_names(cache.root());
    assert_eq!(
        cache.cached(&alias_key(&digest)).unwrap(),
        Some(entry.clone())
    );
    assert_eq!(
        fs::read_to_string(index_path(cache.root())).unwrap(),
        legacy,
        "a legacy index must not be rewritten"
    );
    assert_eq!(file_names(cache.root()), before);
}

#[test]
fn a_hostile_index_entry_is_never_used_or_rewritten() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let digest = pin(BYTES);
    let entry = first_entry(&cache, &digest);
    let hostile = cache.root().join("hostile#mod#1#0000000.zip");
    fs::write(&hostile, b"other bytes").unwrap();
    record(
        cache.root(),
        &ArchivePin::blake3(&digest).unwrap(),
        &name_of(&hostile),
    );

    assert_eq!(
        cache.cached(&alias_key(&digest)).unwrap(),
        Some(entry.clone()),
        "indexed bytes must be re-verified before use"
    );
    let unchanged = ArchiveIndex::read(cache.root()).unwrap();
    let pin = ArchivePin::blake3(&digest).unwrap();
    assert_eq!(
        unchanged.file_name(&pin),
        Some(name_of(&hostile).as_str()),
        "a lookup leaves the hostile record for a writing path to replace"
    );
    assert_eq!(fs::read(&hostile).unwrap(), b"other bytes");
}
