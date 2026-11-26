use super::super::{NexusResolution, ReleaseAcquired, ReleaseResolver};
use crate::acquisition::{
    ArchiveCache, CacheKey, CacheUse, FetchError, GitAcquired, GitCacheKey, NexusApiKey,
    NexusDownloadUrl, NexusFile, Origin, PlannedAction, SourceBody, SourceCandidate, SourcePolicy,
    SourceTransport,
};
use crate::buckets::{BucketPackage, PackageManifest, PackageSource};
use crate::progress::{CancellationToken, OperationControl};
use anyhow::{bail, Result};
use serde_json::json;
use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io::{self, Cursor, Read};
use std::path::PathBuf;

pub(super) const FIRST: &str = "https://fixture.invalid/primary.zip";
pub(super) const SECOND: &str = "https://fixture.invalid/mirror.zip";
pub(super) const GIT_MISSING: &str = "fixture/missing";
pub(super) const GIT_PROJECT: &str = "fixture/project";
pub(super) const GIT_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
pub(super) const NEXUS_LABEL: &str =
    "Nexus Mods: cyberpunk2077 mod 6945 file 123456 (NexusMod.zip)";

pub(super) fn pin(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

pub(super) fn release(sources: serde_json::Value, pin: Option<&str>) -> BucketPackage {
    let mut manifest = json!({
        "schema_version": 2,
        "id": "example",
        "game": "test-game",
        "name": "Example",
        "version": "1.0",
        "archive_format": "zip",
        "sources": sources,
    });
    if let Some(pin) = pin {
        manifest["blake3"] = json!(pin);
    }
    let package: PackageManifest = serde_json::from_value(manifest).expect("fixture manifest");
    BucketPackage {
        bucket: "main".to_owned(),
        package,
    }
}

pub(super) fn https_sources() -> serde_json::Value {
    json!([
        {"kind": "https", "url": FIRST},
        {"kind": "https", "url": SECOND}
    ])
}

pub(super) fn nexus_source(file_id: u64, file_name: &str) -> serde_json::Value {
    json!({
        "kind": "nexus",
        "game_domain": "cyberpunk2077",
        "mod_id": 6945,
        "file_id": file_id,
        "file_name": file_name
    })
}

enum Reply {
    Body(Vec<u8>),
    Failure(FetchError),
}

#[derive(Default)]
pub(super) struct FakeTransport {
    replies: RefCell<BTreeMap<String, VecDeque<Reply>>>,
    calls: RefCell<Vec<String>>,
}

impl FakeTransport {
    fn queue(&self, url: &str, replies: Vec<Reply>) {
        self.replies
            .borrow_mut()
            .entry(url.to_owned())
            .or_default()
            .extend(replies);
    }

    pub(super) fn respond(&self, url: &str, bytes: &[u8]) {
        self.queue(url, vec![Reply::Body(bytes.to_vec())]);
    }

    pub(super) fn fail(&self, url: &str, error: FetchError) {
        self.queue(url, vec![Reply::Failure(error)]);
    }

    pub(super) fn fail_twice(&self, url: &str, error: FetchError) {
        self.queue(
            url,
            vec![Reply::Failure(error.clone()), Reply::Failure(error)],
        );
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}

impl SourceTransport for FakeTransport {
    fn open(
        &self,
        url: &str,
        _policy: &SourcePolicy,
    ) -> std::result::Result<SourceBody, FetchError> {
        self.calls.borrow_mut().push(url.to_owned());
        let mut replies = self.replies.borrow_mut();
        let queue = replies
            .get_mut(url)
            .expect("the acquisition opened an unqueued URL");
        match queue.pop_front().expect("a queued transport reply") {
            Reply::Body(bytes) => Ok(SourceBody::new(Box::new(Cursor::new(bytes)), None)),
            Reply::Failure(error) => Err(error),
        }
    }
}

struct CancelBody {
    token: CancellationToken,
    sent: bool,
}

impl Read for CancelBody {
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        if !self.sent {
            self.sent = true;
            self.token.cancel();
        }
        Err(io::Error::other("fixture body stopped after cancellation"))
    }
}

enum OpenReply {
    Body(Vec<u8>),
    Cancel(CancellationToken),
}

#[derive(Default)]
pub(super) struct FakeResolver {
    git: RefCell<BTreeMap<String, VecDeque<Result<GitAcquired>>>>,
    nexus: RefCell<BTreeMap<u64, VecDeque<NexusResolution>>>,
    open: RefCell<BTreeMap<String, VecDeque<OpenReply>>>,
    opened: RefCell<Vec<String>>,
}

impl FakeResolver {
    pub(super) fn git_ok(&self, repository: &str, path: PathBuf, blake3: String) {
        self.git
            .borrow_mut()
            .entry(repository.to_owned())
            .or_default()
            .push_back(Ok(GitAcquired {
                path,
                blake3,
                origin: Origin::Download,
            }));
    }

    pub(super) fn git_fail(&self, repository: &str, message: &str) {
        self.git
            .borrow_mut()
            .entry(repository.to_owned())
            .or_default()
            .push_back(Err(anyhow::anyhow!(message.to_owned())));
    }

    pub(super) fn nexus_download(&self, file_id: u64, link: NexusDownloadUrl) {
        self.nexus
            .borrow_mut()
            .entry(file_id)
            .or_default()
            .push_back(NexusResolution::Download(link));
    }

    pub(super) fn nexus_refused(&self, file_id: u64, message: &str) {
        self.nexus
            .borrow_mut()
            .entry(file_id)
            .or_default()
            .push_back(NexusResolution::Refused(
                anyhow::anyhow!(message.to_owned()),
            ));
    }

    pub(super) fn open_ok(&self, link: &NexusDownloadUrl, bytes: &[u8]) {
        self.open
            .borrow_mut()
            .entry(link.as_str().to_owned())
            .or_default()
            .push_back(OpenReply::Body(bytes.to_vec()));
    }

    pub(super) fn open_cancel(&self, link: &NexusDownloadUrl, token: CancellationToken) {
        self.open
            .borrow_mut()
            .entry(link.as_str().to_owned())
            .or_default()
            .push_back(OpenReply::Cancel(token));
    }

    pub(super) fn opened(&self) -> Vec<String> {
        self.opened.borrow().clone()
    }
}

impl ReleaseResolver for FakeResolver {
    fn git(
        &self,
        _release: &BucketPackage,
        _cache: &ArchiveCache,
        source: &PackageSource,
        _key: &GitCacheKey<'_>,
        _cache_use: CacheUse,
        _control: &OperationControl,
    ) -> Result<GitAcquired> {
        let PackageSource::Git { repository, .. } = source else {
            bail!("the fake resolver was asked for a non-Git source");
        };
        self.git
            .borrow_mut()
            .get_mut(repository)
            .expect("the Git source was not queued")
            .pop_front()
            .expect("a queued Git result")
    }

    fn nexus(&self, _release: &BucketPackage, source: &PackageSource) -> NexusResolution {
        let PackageSource::Nexus { file_id, .. } = source else {
            return NexusResolution::Refused(anyhow::anyhow!(
                "the fake resolver was asked for a non-Nexus source"
            ));
        };
        self.nexus
            .borrow_mut()
            .get_mut(file_id)
            .expect("the Nexus source was not queued")
            .pop_front()
            .expect("a queued Nexus result")
    }

    fn open_nexus(&self, link: &NexusDownloadUrl) -> Result<SourceBody> {
        let key = link.as_str().to_owned();
        self.opened.borrow_mut().push(key.clone());
        let mut open = self.open.borrow_mut();
        let reply = open
            .get_mut(&key)
            .expect("the Nexus link was not queued")
            .pop_front()
            .expect("a queued Nexus body");
        match reply {
            OpenReply::Body(bytes) => Ok(SourceBody::new(Box::new(Cursor::new(bytes)), None)),
            OpenReply::Cancel(token) => Ok(SourceBody::new(
                Box::new(CancelBody { token, sent: false }),
                None,
            )),
        }
    }

    fn candidate(&self, _release: &BucketPackage, action: PlannedAction) -> SourceCandidate {
        match action {
            PlannedAction::Fetch { candidate } => candidate,
            PlannedAction::Github { expected, .. } => SourceCandidate::fetch(expected),
            _ => SourceCandidate::refused("unknown source", "unknown release source"),
        }
    }
}

pub(super) fn nexus_link(file_id: u64, file_name: &str) -> NexusDownloadUrl {
    let file = NexusFile::new("cyberpunk2077", 6945, file_id, file_name).expect("fixture file");
    let key = NexusApiKey::new("SECRETAPIKEY123").expect("fixture key");
    file.resolve(&key, None, |_request| {
        Ok((
            200,
            Cursor::new(br#"[{"URI":"https://nexus-cdn.com/files/NexusMod.zip"}]"#),
        ))
    })
    .expect("fixture download link")
}

fn cache_entry(cache: &ArchiveCache, release: &BucketPackage, url: &str) -> PathBuf {
    let pin = release.package.pin().expect("a pinned fixture release");
    cache.path(&CacheKey::new(
        &release.package.game,
        &release.package.id,
        &release.package.version,
        url,
        pin,
    ))
}

pub(super) fn seed_cache(
    cache: &ArchiveCache,
    release: &BucketPackage,
    url: &str,
    bytes: &[u8],
) -> PathBuf {
    fs::create_dir_all(cache.root()).expect("cache directory");
    let path = cache_entry(cache, release, url);
    fs::write(&path, bytes).expect("cache entry");
    path
}

pub(super) fn partial_files(cache: &ArchiveCache) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(cache.root()) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "download")
        })
        .collect()
}

pub(super) fn acquire(
    cache: &ArchiveCache,
    release: &BucketPackage,
    cache_use: CacheUse,
    transport: &FakeTransport,
    resolver: &FakeResolver,
    control: &OperationControl,
) -> Result<ReleaseAcquired> {
    cache.acquire_release(
        release,
        cache_use,
        transport,
        SourcePolicy::default(),
        resolver,
        control,
    )
}

pub(super) fn cancelled_control() -> OperationControl {
    let token = CancellationToken::new();
    token.cancel();
    OperationControl::new(token, |_event| {})
}
