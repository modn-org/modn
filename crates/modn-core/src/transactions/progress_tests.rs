use super::catalog_tests::{reference, Fixture};
use super::CatalogRelease;
use crate::acquisition::{ArchiveCache, CacheKey, CacheUse};
use crate::error::{code_of, ErrorCode};
use crate::progress::{CancellationToken, OperationControl, Outcome, ProgressEvent};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read};
use std::path::Path;
use std::sync::{Arc, Mutex};

type Events = Arc<Mutex<Vec<ProgressEvent>>>;

fn recording(token: CancellationToken) -> (OperationControl, Events) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let control = OperationControl::new(token, move |event| {
        sink.lock().expect("event log").push(event.clone());
    });
    (control, events)
}

fn cancelling_on(
    token: CancellationToken,
    phase: fn(&ProgressEvent) -> bool,
) -> (OperationControl, Events) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let control = OperationControl::new(token.clone(), move |event| {
        sink.lock().expect("event log").push(event.clone());
        if phase(event) {
            token.cancel();
        }
    });
    (control, events)
}

fn is_extract(event: &ProgressEvent) -> bool {
    matches!(event, ProgressEvent::Extract { .. })
}

fn is_stage(event: &ProgressEvent) -> bool {
    matches!(event, ProgressEvent::Stage { .. })
}

fn is_commit(event: &ProgressEvent) -> bool {
    matches!(event, ProgressEvent::Commit { .. })
}

fn snapshot(root: &Path) -> BTreeMap<String, String> {
    fn walk(root: &Path, directory: &Path, files: &mut BTreeMap<String, String>) {
        for entry in fs::read_dir(directory).expect("read dir") {
            let path = entry.expect("entry").path();
            let relative = path
                .strip_prefix(root)
                .expect("prefix")
                .to_string_lossy()
                .replace('\\', "/");
            if path.is_dir() {
                files.insert(format!("{relative}/"), "DIR".to_owned());
                walk(root, &path, files);
            } else {
                files.insert(
                    relative,
                    format!("{:x}", Sha256::digest(fs::read(&path).expect("file"))),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    walk(root, root, &mut files);
    files
}

fn assert_untouched(fixture: &Fixture, before: &BTreeMap<String, String>) {
    assert_eq!(&snapshot(&fixture.game), before);
    assert!(fixture.store.installed(&fixture.game).unwrap().is_empty());
    for table in [
        "managed_packages",
        "managed_files",
        "pending_transactions",
        "pending_links",
        "pending_stacks",
        "pending_stack_packages",
    ] {
        assert_eq!(fixture.count(table), 0, "{table}");
    }
}

fn install(fixture: &mut Fixture, archive: &Path) -> anyhow::Result<()> {
    fixture
        .install_release(archive, CatalogRelease::new(reference("mod"), Vec::new()))
        .map(|_| ())
}

#[test]
fn resolve_cancellation_changes_nothing() {
    let mut fixture = Fixture::new();
    let archive = fixture.archive("mod");
    let before = snapshot(&fixture.game);
    let token = CancellationToken::new();
    token.cancel();
    let (control, events) = recording(token);
    fixture.store.set_control(control.clone());
    assert!(control.begin().is_err());
    assert!(install(&mut fixture, &archive).is_err());
    assert!(events.lock().expect("events").is_empty());
    assert_untouched(&fixture, &before);
}

struct CancellingReader {
    body: Cursor<Vec<u8>>,
    token: CancellationToken,
}

impl Read for CancellingReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.token.cancel();
        self.body.read(buffer)
    }
}

#[test]
fn download_cancellation_leaves_no_cache_entry() {
    let body = b"archive bytes".to_vec();
    let cache = ArchiveCache::new(tempfile::tempdir().expect("cache").keep());
    let digest = format!("{:x}", Sha256::digest(&body));
    let key = CacheKey::new(
        "test-game",
        "mod",
        "1.0",
        "https://example.invalid/mod.zip",
        &digest,
    );
    let token = CancellationToken::new();
    let (control, events) = recording(token.clone());
    let length = body.len() as u64;
    let result = cache.acquire_with_control(
        &key,
        CacheUse::Reuse,
        Some("deps/test-game/mod".to_owned()),
        &control,
        |_| {
            Ok((
                CancellingReader {
                    body: Cursor::new(body),
                    token,
                },
                Some(length),
            ))
        },
    );
    let error = result.expect_err("cancelled download");
    assert_eq!(code_of(&error), Some(ErrorCode::Cancelled));
    assert!(cache.entries(None).unwrap().is_empty());
    let leftovers: Vec<_> = fs::read_dir(cache.root())
        .expect("cache dir")
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains(".download"))
        .collect();
    assert!(leftovers.is_empty(), "partial download left behind");
    let events = events.lock().expect("events");
    assert!(matches!(
        events.first(),
        Some(ProgressEvent::Download { bytes: 0, .. })
    ));
}

#[test]
fn extract_cancellation_changes_nothing() {
    let mut fixture = Fixture::new();
    let archive = fixture.archive("mod");
    let before = snapshot(&fixture.game);
    let (control, events) = cancelling_on(CancellationToken::new(), is_extract);
    fixture.store.set_control(control);
    let error = install(&mut fixture, &archive).expect_err("cancelled install");
    assert_eq!(code_of(&error), Some(ErrorCode::Cancelled));
    assert!(events
        .lock()
        .expect("events")
        .iter()
        .any(|event| matches!(event, ProgressEvent::Extract { .. })));
    assert_untouched(&fixture, &before);
}

#[test]
fn stage_cancellation_changes_nothing() {
    let mut fixture = Fixture::new();
    let archive = fixture.archive("mod");
    let before = snapshot(&fixture.game);
    let (control, events) = cancelling_on(CancellationToken::new(), is_stage);
    fixture.store.set_control(control);
    let error = install(&mut fixture, &archive).expect_err("cancelled install");
    assert_eq!(code_of(&error), Some(ErrorCode::Cancelled));
    assert!(events
        .lock()
        .expect("events")
        .iter()
        .any(|event| matches!(event, ProgressEvent::Stage { .. })));
    assert_untouched(&fixture, &before);
}

#[test]
fn commit_cancellation_is_ignored() {
    let mut fixture = Fixture::new();
    let archive = fixture.archive("mod");
    let (control, events) = cancelling_on(CancellationToken::new(), is_commit);
    fixture.store.set_control(control);
    install(&mut fixture, &archive).expect("a committed install finishes");
    assert!(fixture.game.join("addons/mod.plugin").is_file());
    assert_eq!(fixture.store.installed(&fixture.game).unwrap().len(), 1);
    assert!(events
        .lock()
        .expect("events")
        .iter()
        .any(|event| matches!(event, ProgressEvent::Commit { .. })));
}

#[test]
fn successful_install_reports_every_phase_in_order() {
    let mut fixture = Fixture::new();
    let archive = fixture.archive("mod");
    let body = fs::read(&archive).expect("archive bytes");
    let cache = ArchiveCache::new(tempfile::tempdir().expect("cache").keep());
    let digest = format!("{:x}", Sha256::digest(&body));
    let key = CacheKey::new(
        "test-game",
        "mod",
        "1.0",
        "https://example.invalid/mod.zip",
        &digest,
    );
    let (control, events) = recording(CancellationToken::new());
    control.begin().expect("not cancelled");
    let acquired = cache
        .acquire_with_control(
            &key,
            CacheUse::Reuse,
            Some("deps/test-game/mod".to_owned()),
            &control,
            |_| Ok((Cursor::new(body.clone()), Some(body.len() as u64))),
        )
        .expect("download");
    fixture.store.set_control(control.clone());
    install(&mut fixture, &acquired.path).expect("install");
    control.finish(Outcome::Succeeded, None);

    let phases: Vec<String> = events
        .lock()
        .expect("events")
        .iter()
        .map(|event| {
            serde_json::to_value(event).expect("event JSON")["phase"]
                .as_str()
                .expect("phase")
                .to_owned()
        })
        .collect();
    assert_eq!(
        phases,
        ["resolve", "download", "download", "extract", "stage", "commit", "result"]
    );
    let events = events.lock().expect("events");
    let ProgressEvent::Download { bytes, total, .. } = &events[1] else {
        panic!("second event is a download");
    };
    assert_eq!(*bytes, 0);
    assert_eq!(*total, Some(body.len() as u64));
    assert_eq!(
        events.last(),
        Some(&ProgressEvent::Result {
            outcome: Outcome::Succeeded,
            error: None,
        })
    );
}

#[test]
fn uninstall_cancellation_before_the_commit_changes_nothing() {
    let mut fixture = Fixture::new();
    let archive = fixture.archive("mod");
    install(&mut fixture, &archive).expect("install");
    let installed = fixture.store.installed(&fixture.game).unwrap();
    assert_eq!(installed.len(), 1);
    let before = snapshot(&fixture.game);
    let token = CancellationToken::new();
    token.cancel();
    fixture
        .store
        .set_control(OperationControl::new(token, |_| {}));
    let error = fixture
        .uninstall(&installed[0].id)
        .expect_err("cancelled uninstall");
    assert_eq!(code_of(&error), Some(ErrorCode::Cancelled));
    assert_eq!(snapshot(&fixture.game), before);
    assert_eq!(fixture.store.installed(&fixture.game).unwrap().len(), 1);
}

#[test]
fn uninstall_cancellation_during_the_commit_is_ignored() {
    let mut fixture = Fixture::new();
    let archive = fixture.archive("mod");
    install(&mut fixture, &archive).expect("install");
    let id = fixture.store.installed(&fixture.game).unwrap()[0]
        .id
        .clone();
    let (control, _events) = cancelling_on(CancellationToken::new(), is_commit);
    fixture.store.set_control(control);
    fixture
        .uninstall(&id)
        .expect("a committed uninstall finishes");
    assert!(!fixture.game.join("addons/mod.plugin").exists());
    assert!(fixture.store.installed(&fixture.game).unwrap().is_empty());
}
