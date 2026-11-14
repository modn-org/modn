use super::support::{
    assert_no_secrets, error_of, expires, free_responses, header, nxm_link, premium_responses,
    with_api_key, with_missing_key, Fixture, API_KEY, CDN_URL, FILE_ID, NXM_KEY,
};
use crate::download::nexus_download;
use std::collections::BTreeMap;
use std::fs;

#[test]
fn free_path_downloads_with_the_nxm_grant_and_installs() {
    let fixture = Fixture::new(b"v1");
    let expires = expires();
    nexus_download::set_responses(free_responses(
        FILE_ID,
        expires,
        CDN_URL,
        &fixture.archive_bytes("1"),
    ));
    with_api_key(|| {
        fixture.run(&[
            "install",
            "nexusmod",
            "--path",
            fixture.path(),
            "--yes",
            "--nxm",
            &nxm_link(FILE_ID, expires),
        ])
    })
    .expect("free-path install");
    assert_eq!(
        fs::read(fixture.game.join("addons/nexus.plugin")).expect("installed file"),
        b"v1"
    );
    let requests = nexus_download::requests();
    let api = requests
        .iter()
        .find(|request| request.url.starts_with("https://api.nexusmods.com/"))
        .expect("API request");
    assert!(api.url.contains(&format!("key={NXM_KEY}")), "{}", api.url);
    assert!(
        api.url.contains(&format!("expires={expires}")),
        "{}",
        api.url
    );
    assert_eq!(header(api, "apikey"), Some(API_KEY));
    assert_eq!(header(api, "application-name"), Some("Modn"));
    assert_eq!(
        header(api, "application-version"),
        Some(env!("CARGO_PKG_VERSION"))
    );
    let cdn = requests
        .iter()
        .find(|request| request.url == CDN_URL)
        .expect("CDN request");
    assert_eq!(header(cdn, "apikey"), None);
}

#[test]
fn premium_path_downloads_without_a_grant_and_installs() {
    let fixture = Fixture::new(b"v1");
    nexus_download::set_responses(premium_responses(
        FILE_ID,
        CDN_URL,
        &fixture.archive_bytes("1"),
    ));
    with_api_key(|| fixture.run(&["install", "nexusmod", "--path", fixture.path(), "--yes"]))
        .expect("premium install");
    assert_eq!(
        fs::read(fixture.game.join("addons/nexus.plugin")).expect("installed file"),
        b"v1"
    );
    let requests = nexus_download::requests();
    let api = requests
        .iter()
        .find(|request| request.url.starts_with("https://api.nexusmods.com/"))
        .expect("API request");
    assert!(!api.url.contains('?'), "{}", api.url);
    assert_eq!(header(api, "apikey"), Some(API_KEY));
    let cdn = requests
        .iter()
        .find(|request| request.url == CDN_URL)
        .expect("CDN request");
    assert_eq!(header(cdn, "apikey"), None);
}

#[test]
fn second_install_reuses_the_verified_cache_without_requests_or_key() {
    let fixture = Fixture::new(b"v1");
    let expires = expires();
    nexus_download::set_responses(free_responses(
        FILE_ID,
        expires,
        CDN_URL,
        &fixture.archive_bytes("1"),
    ));
    with_api_key(|| {
        fixture.run(&[
            "install",
            "nexusmod",
            "--path",
            fixture.path(),
            "--yes",
            "--nxm",
            &nxm_link(FILE_ID, expires),
        ])
    })
    .expect("first install");
    fixture.reset_state_and_game();
    nexus_download::set_responses(BTreeMap::new());
    with_missing_key(|| fixture.run(&["install", "nexusmod", "--path", fixture.path(), "--yes"]))
        .expect("cached install");
    assert!(nexus_download::requests().is_empty());
    assert_eq!(
        fs::read(fixture.game.join("addons/nexus.plugin")).expect("installed file"),
        b"v1"
    );
}

#[test]
fn a_pin_mismatch_from_the_cdn_is_refused() {
    let fixture = Fixture::new(b"v1");
    let expires = expires();
    nexus_download::set_responses(free_responses(FILE_ID, expires, CDN_URL, b"tampered"));
    let message = with_api_key(|| {
        error_of(fixture.run(&[
            "install",
            "nexusmod",
            "--path",
            fixture.path(),
            "--yes",
            "--nxm",
            &nxm_link(FILE_ID, expires),
        ]))
    });
    assert!(message.contains("BLAKE3 check failed"), "{message}");
    assert_no_secrets(&message);
    assert!(!fixture.game.join("addons").exists());
}

#[test]
fn update_downloads_the_new_release_and_reuses_its_cache_without_a_key() {
    let fixture = Fixture::new(b"v1");
    let first = expires();
    nexus_download::set_responses(free_responses(
        FILE_ID,
        first,
        CDN_URL,
        &fixture.archive_bytes("1"),
    ));
    with_api_key(|| {
        fixture.run(&[
            "install",
            "nexusmod",
            "--path",
            fixture.path(),
            "--yes",
            "--nxm",
            &nxm_link(FILE_ID, first),
        ])
    })
    .expect("install v1");
    fixture.release("1", FILE_ID, b"v1", false);
    let second_file = FILE_ID + 1;
    let second_cdn = "https://nexus-cdn.com/files/NexusMod-2.zip?token=SECRETCDN789";
    fixture.release("2", second_file, b"v2", true);
    let second = expires();
    nexus_download::set_responses(free_responses(
        second_file,
        second,
        second_cdn,
        &fixture.archive_bytes("2"),
    ));
    with_api_key(|| {
        fixture.run(&[
            "update",
            "nexusmod",
            "--path",
            fixture.path(),
            "--yes",
            "--nxm",
            &nxm_link(second_file, second),
        ])
    })
    .expect("update to v2");
    assert_eq!(
        fs::read(fixture.game.join("addons/nexus.plugin")).expect("updated file"),
        b"v2"
    );
    assert!(nexus_download::requests().iter().any(|request| {
        request
            .url
            .contains(&format!("files/{second_file}/download_link.json"))
    }));

    fixture.release("2", second_file, b"v2", false);
    let third_file = second_file + 1;
    let third_cdn = "https://nexus-cdn.com/files/NexusMod-3.zip?token=SECRETCDN789";
    fixture.release("3", third_file, b"v3", true);
    let third = expires();
    nexus_download::set_responses(free_responses(
        third_file,
        third,
        third_cdn,
        &fixture.archive_bytes("3"),
    ));
    with_api_key(|| {
        fixture.run(&[
            "update",
            "nexusmod",
            "--path",
            fixture.path(),
            "--dry-run",
            "--download",
            "--nxm",
            &nxm_link(third_file, third),
        ])
    })
    .expect("download v3 into the cache");
    assert_eq!(
        fs::read(fixture.game.join("addons/nexus.plugin")).expect("installed file"),
        b"v2"
    );

    nexus_download::set_responses(BTreeMap::new());
    with_missing_key(|| fixture.run(&["update", "nexusmod", "--path", fixture.path(), "--yes"]))
        .expect("cached update");
    assert!(nexus_download::requests().is_empty());
    assert_eq!(
        fs::read(fixture.game.join("addons/nexus.plugin")).expect("updated file"),
        b"v3"
    );
}

#[test]
fn a_dry_run_without_download_needs_no_key_or_request() {
    let fixture = Fixture::new(b"v1");
    nexus_download::set_responses(BTreeMap::new());
    with_missing_key(|| {
        fixture.run(&["install", "nexusmod", "--path", fixture.path(), "--dry-run"])
    })
    .expect("dry run");
    assert!(nexus_download::requests().is_empty());
    assert!(!fixture.game.join("addons").exists());
}
