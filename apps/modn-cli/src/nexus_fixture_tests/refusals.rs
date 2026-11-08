use super::support::{
    api_link_body, api_url, assert_no_secrets, error_of, expires, nxm_link, response_map,
    with_api_key, with_missing_key, Fixture, CDN_URL, FILE_ID, MOD_ID,
};
use crate::download::nexus_download::{self, FixtureResponse};
use crate::nexus_auth;
use std::collections::BTreeMap;

#[test]
fn nexus_status_errors_are_explained_without_secrets() {
    for (status, expected) in [
        (401, "rejected the API key"),
        (403, "free account"),
        (410, "download link expired"),
        (429, "rate limit"),
    ] {
        let fixture = Fixture::new(b"v1");
        let grant = (status != 403).then(expires);
        nexus_download::set_responses(response_map([(
            api_url(FILE_ID, grant),
            FixtureResponse::status(status),
        )]));
        let link = grant.map(|expires| nxm_link(FILE_ID, expires));
        let mut args = vec!["install", "nexusmod", "--path", fixture.path(), "--yes"];
        if let Some(link) = &link {
            args.extend(["--nxm", link.as_str()]);
        }
        let message = with_api_key(|| error_of(fixture.run(&args)));
        assert!(message.contains(expected), "{status}: {message}");
        assert_no_secrets(&message);
        assert!(!fixture.game.join("addons").exists());
        assert!(nexus_download::requests()
            .iter()
            .all(|request| { request.url.starts_with("https://api.nexusmods.com/") }));
    }
}

#[test]
fn an_unmatched_nxm_link_is_refused() {
    let fixture = Fixture::new(b"v1");
    nexus_download::set_responses(BTreeMap::new());
    let message = with_missing_key(|| {
        error_of(fixture.run(&[
            "install",
            "nexusmod",
            "--path",
            fixture.path(),
            "--yes",
            "--nxm",
            &nxm_link(FILE_ID + 1, expires()),
        ]))
    });
    assert!(
        message.contains(&format!("nxm link for mod {MOD_ID} file {}", FILE_ID + 1)),
        "{message}"
    );
    assert!(nexus_download::requests().is_empty());
    assert!(!fixture.game.join("addons").exists());
}

#[test]
fn a_missing_api_key_is_refused() {
    let fixture = Fixture::new(b"v1");
    nexus_download::set_responses(BTreeMap::new());
    let message = with_missing_key(|| {
        error_of(fixture.run(&["install", "nexusmod", "--path", fixture.path(), "--yes"]))
    });
    assert_eq!(message, nexus_auth::MISSING_KEY);
    assert!(nexus_download::requests().is_empty());
    assert!(!fixture.game.join("addons").exists());
}

#[test]
fn an_api_redirect_is_refused() {
    let fixture = Fixture::new(b"v1");
    nexus_download::set_responses(response_map([(
        api_url(FILE_ID, None),
        FixtureResponse::redirect("https://api.nexusmods.com/v1/other"),
    )]));
    let message = with_api_key(|| {
        error_of(fixture.run(&["install", "nexusmod", "--path", fixture.path(), "--yes"]))
    });
    assert!(message.contains("must not redirect"), "{message}");
    assert_no_secrets(&message);
    assert!(!fixture.game.join("addons").exists());
}

#[test]
fn a_cdn_redirect_to_a_non_nexus_host_is_refused() {
    let fixture = Fixture::new(b"v1");
    nexus_download::set_responses(response_map([
        (
            api_url(FILE_ID, None),
            FixtureResponse::ok(api_link_body(CDN_URL)),
        ),
        (
            CDN_URL.to_owned(),
            FixtureResponse::redirect("https://evil.invalid/NexusMod.zip"),
        ),
    ]));
    let message = with_api_key(|| {
        error_of(fixture.run(&["install", "nexusmod", "--path", fixture.path(), "--yes"]))
    });
    assert!(message.contains("not a Nexus host"), "{message}");
    assert_no_secrets(&message);
    assert!(!fixture.game.join("addons").exists());
}
