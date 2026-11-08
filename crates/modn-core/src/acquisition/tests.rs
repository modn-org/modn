use super::*;
use std::cell::Cell;
use std::io::Cursor;
use tempfile::tempdir;

const URL: &str = "https://example.invalid/releases/v1.0/fixture-1.0.zip?raw=1";

#[test]
fn aliases_reuse_verified_bytes_and_repair_corruption() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().to_owned());
    let digest = sha(b"shared bytes");
    let first = cache
        .acquire(&key(&digest), CacheUse::Reuse, |_| {
            Ok(Cursor::new(b"shared bytes"))
        })
        .unwrap();
    let alias = CacheKey::new(
        "other-game",
        "alias",
        "2",
        "https://other.invalid/a.zip",
        &digest,
    );
    let reused = cache
        .acquire(&alias, CacheUse::Reuse, |_| -> Result<Cursor<Vec<u8>>> {
            bail!("alias must not download")
        })
        .unwrap();
    assert_eq!(reused.path, first.path);
    assert_eq!(cache.entries(None).unwrap().len(), 1);
    fs::write(&first.path, b"corrupt").unwrap();
    assert!(cache.cached(&alias).unwrap().is_none());
    let repaired = cache
        .acquire(&alias, CacheUse::Reuse, |_| {
            Ok(Cursor::new(b"shared bytes"))
        })
        .unwrap();
    assert_eq!(fs::read(repaired.path).unwrap(), b"shared bytes");
}

fn sha(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn key(sha256: &str) -> CacheKey<'_> {
    CacheKey::new("test-game", "fixture", "1.0 beta/2", URL, sha256)
}

#[test]
fn entry_names_follow_the_scoop_layout() {
    let cache = ArchiveCache::new(PathBuf::from("cache"));
    let name = cache
        .path(&key("0"))
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let url_hash = &sha(URL.as_bytes())[..7];
    assert_eq!(name, format!("test-game#fixture#1.0_beta_2#{url_hash}.zip"));
}

#[test]
fn download_is_verified_then_reused() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let body = b"archive bytes".to_vec();
    let digest = sha(&body);
    let fetches = Cell::new(0);
    let fetch = |url: &str| {
        assert_eq!(url, URL);
        fetches.set(fetches.get() + 1);
        Ok(Cursor::new(body.clone()))
    };

    let first = cache
        .acquire(&key(&digest), CacheUse::Reuse, fetch)
        .unwrap();
    assert_eq!(first.origin, Origin::Download);
    assert_eq!(fs::read(&first.path).unwrap(), body);
    assert_eq!(cache.entries(None).unwrap().len(), 1);

    let second = cache
        .acquire(&key(&digest), CacheUse::Reuse, fetch)
        .unwrap();
    assert_eq!(
        second,
        Acquired {
            path: first.path.clone(),
            origin: Origin::Cache
        }
    );
    assert_eq!(fetches.get(), 1);

    let refreshed = cache
        .acquire(&key(&digest), CacheUse::Refresh, fetch)
        .unwrap();
    assert_eq!(refreshed.origin, Origin::Download);
    assert_eq!(fetches.get(), 2);
}

#[test]
fn hash_mismatch_leaves_no_cache_entry() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let expected = sha(b"pinned");
    let error = cache
        .acquire(&key(&expected), CacheUse::Reuse, |_| {
            Ok(Cursor::new(b"other".to_vec()))
        })
        .unwrap_err()
        .to_string();
    assert!(error.contains("hash check failed"), "{error}");
    assert!(error.contains(&format!("Expected: {expected}")), "{error}");
    assert!(
        error.contains(&format!("Actual:   {}", sha(b"other"))),
        "{error}"
    );
    assert!(cache.entries(None).unwrap().is_empty());
    assert_eq!(fs::read_dir(cache.root()).unwrap().count(), 0);

    let error = cache
        .acquire(
            &key(&expected),
            CacheUse::Reuse,
            |_| -> Result<Cursor<Vec<u8>>> { bail!("connection refused") },
        )
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("download failed"),
        "{error:#}"
    );
    assert_eq!(fs::read_dir(cache.root()).unwrap().count(), 0);
}

#[test]
fn a_tampered_entry_is_never_used_and_is_replaced() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let body = b"genuine".to_vec();
    let digest = sha(&body);
    let path = cache
        .acquire(&key(&digest), CacheUse::Reuse, |_| {
            Ok(Cursor::new(body.clone()))
        })
        .unwrap()
        .path;
    fs::write(&path, b"tampered").unwrap();
    assert_eq!(cache.cached(&key(&digest)).unwrap(), None);
    assert_eq!(fs::read(&path).unwrap(), b"tampered");
    let again = cache
        .acquire(&key(&digest), CacheUse::Reuse, |_| {
            Ok(Cursor::new(body.clone()))
        })
        .unwrap();
    assert_eq!(again.origin, Origin::Download);
    assert_eq!(again.path, path);
    assert_eq!(fs::read(again.path).unwrap(), body);
}

#[test]
fn oversized_body_is_refused_without_a_cache_entry() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let body = vec![0; 9];
    let digest = sha(&body);
    let error = cache
        .acquire_limited(&key(&digest), CacheUse::Reuse, 8, |_| Ok(io::repeat(0)))
        .unwrap_err()
        .to_string();
    assert!(error.contains("download exceeds 8 bytes"), "{error}");
    assert_eq!(fs::read_dir(cache.root()).unwrap().count(), 0);
}

#[test]
fn entries_filter_by_mod_and_remove_only_listed_files() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    for (mod_id, body) in [("alpha", b"a".as_slice()), ("beta", b"b".as_slice())] {
        let digest = sha(body);
        let key = CacheKey {
            mod_id,
            ..key(&digest)
        };
        cache
            .acquire(&key, CacheUse::Reuse, |_| Ok(Cursor::new(body.to_vec())))
            .unwrap();
    }
    for name in [
        "unrelated.txt",
        "notes#alpha#draft#keep.txt",
        "test-game#alpha#1.0#XYZ1234.zip",
        "test-game#alpha#1.0#abcdef0.zip.1234.download",
        "test-game#alpha#1.0#abcdef0.zip.bak",
        "test-game#alpha#1.0#abcdef0.zip.old",
    ] {
        fs::write(cache.root().join(name), b"keep").unwrap();
    }
    let alpha = cache.entries(Some("alpha")).unwrap();
    assert_eq!(alpha.len(), 1);
    assert_eq!(alpha[0].version, "1.0_beta_2");
    assert_eq!(alpha[0].bytes, 1);
    assert_eq!(cache.entries(None).unwrap().len(), 2);

    assert_eq!(cache.remove(Some("alpha")).unwrap().len(), 1);
    assert_eq!(cache.remove(None).unwrap().len(), 1);
    // Six unrelated files kept, plus the digest index.
    assert_eq!(fs::read_dir(cache.root()).unwrap().count(), 7);
}

#[test]
fn another_attempts_partial_file_is_never_touched() {
    let dir = tempdir().unwrap();
    let cache = ArchiveCache::new(dir.path().join("cache"));
    let body = b"verified".to_vec();
    let digest = sha(&body);
    fs::create_dir_all(cache.root()).unwrap();
    let entry = cache.path(&key(&digest));
    let other = cache.root().join(format!(
        "{}.download",
        entry.file_name().unwrap().to_string_lossy()
    ));
    fs::write(&other, b"still downloading").unwrap();
    let acquired = cache
        .acquire(&key(&digest), CacheUse::Reuse, |_| {
            Ok(Cursor::new(body.clone()))
        })
        .unwrap();
    assert_eq!(fs::read(&acquired.path).unwrap(), body);
    assert_eq!(fs::read(&other).unwrap(), b"still downloading");
    let error = cache
        .acquire(&key(&digest), CacheUse::Refresh, |_| {
            Ok(Cursor::new(b"bad".to_vec()))
        })
        .unwrap_err();
    assert!(error.to_string().contains("hash check failed"));
    assert_eq!(fs::read(&other).unwrap(), b"still downloading");
    assert_eq!(fs::read(&acquired.path).unwrap(), body);
    assert_eq!(fs::read_dir(cache.root()).unwrap().count(), 3);
}
