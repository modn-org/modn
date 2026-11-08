use super::super::{hex, ArchiveCache, CacheKey, CacheUse};
use super::{index_path, ArchiveIndex, ArchivePin};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

const ALIAS_URL: &str = "https://alias.invalid/alias.zip";
const BYTES: &[u8] = b"shared verified bytes";

fn sha(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
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
    let digest = sha(BYTES);
    let pin = ArchivePin::sha256(&digest).unwrap();
    assert_eq!(pin.encode(), format!("sha256:{digest}"));
    assert_eq!(ArchivePin::decode(&pin.encode()), Some(pin));
    assert!(ArchivePin::sha256(&digest.to_uppercase()).is_none());
    assert!(ArchivePin::sha256("abc").is_none());
    assert!(ArchivePin::decode(&format!("blake3:{digest}")).is_none());
}

#[test]
fn lookup_uses_the_index_and_matches_the_scan_result() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let digest = sha(BYTES);
    let first = first_entry(&cache, &digest);
    let second = second_entry(&cache, &digest);
    assert!(first < second, "listing order must prefer the first entry");

    let pin = ArchivePin::sha256(&digest).unwrap();
    record(cache.root(), &pin, &name_of(&second));
    assert_eq!(
        cache.cached(&alias_key(&digest)).unwrap(),
        Some(second.clone()),
        "the index decides which verified entry is returned"
    );

    fs::remove_file(index_path(cache.root())).unwrap();
    assert_eq!(
        cache.cached(&alias_key(&digest)).unwrap(),
        Some(first.clone()),
        "a scan returns the first verified entry in listing order"
    );
    let rebuilt = ArchiveIndex::read(cache.root()).expect("scan rebuilds the index");
    let first_name = name_of(&first);
    assert_eq!(rebuilt.file_name(&pin), Some(first_name.as_str()));
}

#[test]
fn a_missing_index_falls_back_to_a_scan_and_is_rebuilt() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let digest = sha(BYTES);
    let entry = first_entry(&cache, &digest);
    let pin = ArchivePin::sha256(&digest).unwrap();
    assert!(ArchiveIndex::read(cache.root()).is_some());
    fs::remove_file(index_path(cache.root())).unwrap();

    assert_eq!(
        cache.cached(&alias_key(&digest)).unwrap(),
        Some(entry.clone())
    );
    let rebuilt = ArchiveIndex::read(cache.root()).expect("missing index is rebuilt");
    let entry_name = name_of(&entry);
    assert_eq!(rebuilt.file_name(&pin), Some(entry_name.as_str()));
}

#[test]
fn a_corrupt_index_falls_back_to_a_scan_and_is_rebuilt() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let digest = sha(BYTES);
    let entry = first_entry(&cache, &digest);
    let pin = ArchivePin::sha256(&digest).unwrap();
    let corrupt: [&[u8]; 4] = [
        b"garbage bytes",
        br#"{"version":1,"entries":{"sha256:"#,
        br#"{"version":2,"entries":{}}"#,
        br#"{"version":1,"entries":{},"future":true}"#,
    ];

    for payload in corrupt {
        fs::write(index_path(cache.root()), payload).unwrap();
        assert_eq!(
            cache.cached(&alias_key(&digest)).unwrap(),
            Some(entry.clone()),
            "corrupt index {:?} must not hide the entry",
            String::from_utf8_lossy(payload)
        );
        let rebuilt = ArchiveIndex::read(cache.root()).expect("corrupt index is rebuilt");
        let entry_name = name_of(&entry);
        assert_eq!(rebuilt.file_name(&pin), Some(entry_name.as_str()));
    }
}

#[test]
fn stale_index_entries_are_dropped_and_never_returned() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let digest = sha(BYTES);
    let first = first_entry(&cache, &digest);
    let second = second_entry(&cache, &digest);
    let pin = ArchivePin::sha256(&digest).unwrap();

    record(cache.root(), &pin, &name_of(&second));
    fs::remove_file(&second).unwrap();
    assert_eq!(
        cache.cached(&alias_key(&digest)).unwrap(),
        Some(first.clone())
    );
    let rebuilt = ArchiveIndex::read(cache.root()).unwrap();
    let first_name = name_of(&first);
    assert_eq!(rebuilt.file_name(&pin), Some(first_name.as_str()));

    fs::write(&second, b"tampered").unwrap();
    record(cache.root(), &pin, &name_of(&second));
    assert_eq!(cache.cached(&alias_key(&digest)).unwrap(), Some(first));
}

#[test]
fn a_tampered_indexed_entry_is_never_returned() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let digest = sha(BYTES);
    let entry = first_entry(&cache, &digest);
    let pin = ArchivePin::sha256(&digest).unwrap();
    fs::write(&entry, b"tampered").unwrap();

    assert_eq!(cache.cached(&alias_key(&digest)).unwrap(), None);
    let rebuilt = ArchiveIndex::read(cache.root()).unwrap();
    assert_eq!(rebuilt.file_name(&pin), None, "the stale pin is dropped");
}

#[test]
fn acquire_records_the_stored_entry_in_the_index() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let digest = sha(BYTES);
    let entry = first_entry(&cache, &digest);
    let name = name_of(&entry);
    let text = fs::read_to_string(index_path(cache.root())).unwrap();
    assert!(
        text.contains(&format!("\"sha256:{digest}\":\"{name}\"")),
        "{text}"
    );

    let entries = cache.entries(None).unwrap();
    assert_eq!(entries.len(), 1, "the index file is not a cache entry");
    assert_eq!(entries[0].path, entry);
}
