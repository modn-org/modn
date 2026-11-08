use super::{NexusApiKey, NexusDownloadGrant, NexusDownloadUrl, NexusFile};
use serde_json::json;
use std::io::Cursor;

const API_KEY: &str = "api-key_secret";
const GRANT_KEY: &str = "grant-key_secret";
const EXPIRES: u64 = 1_760_000_000;

fn file() -> NexusFile {
    NexusFile::new("cyberpunk2077", 123, 456, "mod.zip").unwrap()
}

fn api_key() -> NexusApiKey {
    NexusApiKey::new(API_KEY).unwrap()
}

fn grant() -> NexusDownloadGrant {
    NexusDownloadGrant::new(GRANT_KEY, EXPIRES).unwrap()
}

#[test]
fn accepts_valid_file_selectors() {
    for (domain, mod_id, file_id, name) in [
        ("cyberpunk2077", 1, 1, "mod.zip"),
        ("balatro", 9_999_999_999, 9_999_999_999, "my mod archive.7z"),
        ("a", 42, 43, "README"),
    ] {
        let file = NexusFile::new(domain, mod_id, file_id, name).unwrap();
        assert_eq!(
            file.cache_url(),
            format!("nexus://{domain}/mods/{mod_id}/files/{file_id}/{name}")
        );
    }
}

#[test]
fn rejects_invalid_game_domains() {
    let long = "a".repeat(65);
    for domain in [
        "",
        "-cyberpunk",
        "cyberpunk-",
        "Cyberpunk2077",
        "cyber_punk",
        "cyber punk",
        long.as_str(),
    ] {
        assert!(NexusFile::new(domain, 1, 1, "mod.zip").is_err(), "{domain}");
    }
}

#[test]
fn rejects_out_of_range_ids() {
    for id in [0, 10_000_000_000] {
        assert!(NexusFile::new("cyberpunk2077", id, 1, "mod.zip").is_err());
        assert!(NexusFile::new("cyberpunk2077", 1, id, "mod.zip").is_err());
    }
}

#[test]
fn rejects_invalid_file_names() {
    let long = "a".repeat(256);
    for name in [
        "",
        ".",
        "..",
        "a/b",
        "a\\b",
        "a#b",
        "a?b",
        "a\"b",
        "a<b",
        "a>b",
        "a|b",
        "a:b",
        "a\nb",
        "a\tb",
        long.as_str(),
    ] {
        assert!(
            NexusFile::new("cyberpunk2077", 1, 1, name).is_err(),
            "{name:?}"
        );
    }
}

#[test]
fn cache_url_is_exact_and_secret_free() {
    let url = file().cache_url();
    assert_eq!(url, "nexus://cyberpunk2077/mods/123/files/456/mod.zip");
    assert!(!url.contains(API_KEY));
    assert!(!url.contains(GRANT_KEY));
    assert!(!url.contains(&EXPIRES.to_string()));
}

#[test]
fn secrets_reject_empty_and_control_values() {
    for value in ["", "\n", "a\tb", "a\u{7f}b"] {
        assert!(NexusApiKey::new(value).is_err());
        assert!(NexusDownloadGrant::new(value, 1).is_err());
    }
    assert_eq!(api_key().expose_secret(), API_KEY);
    assert_eq!(grant().expose_secret(), GRANT_KEY);
}

#[test]
fn premium_request_url_has_no_query_and_expected_headers() {
    let request = file().download_link_request(&api_key(), None);
    assert_eq!(
        request.url_for_transport(),
        "https://api.nexusmods.com/v1/games/cyberpunk2077/mods/123/files/456/download_link.json"
    );
    let headers = request.headers();
    assert_eq!(headers.len(), 3);
    assert_eq!(headers[0], ("apikey".to_owned(), API_KEY.to_owned()));
    assert_eq!(
        headers[1],
        ("Application-Name".to_owned(), "Modn".to_owned())
    );
    assert_eq!(
        headers[2],
        (
            "Application-Version".to_owned(),
            env!("CARGO_PKG_VERSION").to_owned()
        )
    );
}

#[test]
fn free_request_url_percent_encodes_the_grant_key() {
    let grant = NexusDownloadGrant::new("a b/c+d=e", 1_700_000_000).unwrap();
    let request = file().download_link_request(&api_key(), Some(&grant));
    assert_eq!(
        request.url_for_transport(),
        concat!(
            "https://api.nexusmods.com/v1/games/cyberpunk2077/mods/123/files/456/download_link.json",
            "?key=a%20b%2Fc%2Bd%3De&expires=1700000000"
        )
    );
}

#[test]
fn debug_redacts_api_key_secret() {
    let debug = format!("{:?}", api_key());
    assert!(!debug.contains(API_KEY));
    assert!(debug.contains("<redacted>"));
}

#[test]
fn debug_redacts_grant_key() {
    let debug = format!("{:?}", grant());
    assert!(!debug.contains(GRANT_KEY));
    assert!(debug.contains("<redacted>"));
}

#[test]
fn debug_redacts_request_secrets_and_grant_query() {
    let request = file().download_link_request(&api_key(), Some(&grant()));
    let debug = format!("{request:?}");
    assert!(!debug.contains(API_KEY));
    assert!(!debug.contains(GRANT_KEY));
    assert!(!debug.contains(&EXPIRES.to_string()));
    assert!(debug.contains("<redacted>"));
}

#[test]
fn debug_redacts_cdn_query() {
    let url = NexusDownloadUrl::parse(
        "https://cf-files.nexus-cdn.com/123/Mod.7z?md5=signed-token&expires=1",
    )
    .unwrap();
    let debug = format!("{url:?}");
    assert!(!debug.contains("signed-token"));
    assert!(!debug.contains("md5="));
    assert!(debug.contains("<redacted>"));
    assert!(debug.contains("https://cf-files.nexus-cdn.com/123/Mod.7z"));
}

#[test]
fn maps_status_codes_to_messages_without_secrets() {
    let api_key = api_key();
    let grant = grant();
    let cases = [
        (400, "invalid or expired"),
        (401, "API key"),
        (403, "--nxm"),
        (404, "not found"),
        (410, "expired"),
        (429, "rate limit"),
    ];
    for (status, expected) in cases {
        let error = file()
            .resolve(&api_key, Some(&grant), |_| {
                Ok((status, Cursor::new(Vec::new())))
            })
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{status}: {error}");
        assert!(!error.contains(API_KEY), "{status}: {error}");
        assert!(!error.contains(GRANT_KEY), "{status}: {error}");
        assert!(!error.contains(&EXPIRES.to_string()), "{status}: {error}");
    }
}

#[test]
fn maps_other_statuses_to_a_generic_message() {
    let error = file()
        .resolve(&api_key(), None, |_| Ok((503, Cursor::new(Vec::new()))))
        .unwrap_err()
        .to_string();
    assert!(error.contains("503"));
    assert!(!error.contains(API_KEY));
}

#[test]
fn resolves_the_first_download_link() {
    let body = json!([
        {
            "name": "Main",
            "short_name": "main",
            "URI": "https://cf-files.nexus-cdn.com/123/456/Mod.7z?md5=abcdef&expires=1"
        },
        {
            "name": "Other",
            "short_name": "other",
            "URI": "https://supporter-files.nexus-cdn.com/9/9/other.7z"
        }
    ])
    .to_string();
    let url = file()
        .resolve(&api_key(), None, |request| {
            assert_eq!(
                request.url_for_transport(),
                "https://api.nexusmods.com/v1/games/cyberpunk2077/mods/123/files/456/download_link.json"
            );
            Ok((200, Cursor::new(body)))
        })
        .unwrap();
    assert_eq!(
        url.as_str(),
        "https://cf-files.nexus-cdn.com/123/456/Mod.7z?md5=abcdef&expires=1"
    );
}

#[test]
fn rejects_empty_invalid_and_oversized_responses() {
    for body in [
        "[]".to_owned(),
        "not JSON".to_owned(),
        "{}".to_owned(),
        json!([{"name": "Main"}]).to_string(),
    ] {
        let message = body.clone();
        assert!(
            file()
                .resolve(&api_key(), None, |_| Ok((200, Cursor::new(body))))
                .is_err(),
            "{message}"
        );
    }
    let oversized = vec![b' '; 1024 * 1024 + 1];
    let error = file()
        .resolve(&api_key(), None, |_| Ok((200, Cursor::new(oversized))))
        .unwrap_err()
        .to_string();
    assert!(error.contains("size limit"));
}

#[test]
fn rejects_unsafe_download_link_uris() {
    for uri in [
        "http://cf-files.nexus-cdn.com/123/Mod.7z",
        "https://evil.example.com/123/Mod.7z",
    ] {
        let body = json!([{"name": "Main", "short_name": "main", "URI": uri}]).to_string();
        assert!(
            file()
                .resolve(&api_key(), None, |_| Ok((200, Cursor::new(body))))
                .is_err(),
            "{uri}"
        );
    }
}

#[test]
fn rejects_unsafe_cdn_urls() {
    for value in [
        "http://cf-files.nexus-cdn.com/123/Mod.7z",
        "https://user@cf-files.nexus-cdn.com/123/Mod.7z",
        "https://cf-files.nexus-cdn.com:443/123/Mod.7z",
        "https://cf-files.nexus-cdn.com/123/Mod.7z#fragment",
        "https://cf-files.nexus-cdn.com/123\\Mod.7z",
        "https://cf-files.nexus-cdn.com/123/Mod .7z",
        "https://evil.example.com/123/Mod.7z",
        "https://evilnexus-cdn.com/123/Mod.7z",
        "https://cf-files.nexus-cdn.com/",
        "https://cf-files.nexus-cdn.com",
        "https://cf-files..nexus-cdn.com/123/Mod.7z",
        "https:///123/Mod.7z",
    ] {
        assert!(NexusDownloadUrl::parse(value).is_err(), "{value}");
    }
}

#[test]
fn accepts_nexus_cdn_urls() {
    for value in [
        "https://cf-files.nexus-cdn.com/123/456/Mod.7z",
        "https://supporter-files.nexus-cdn.com/123/456/Mod.7z?md5=abc&expires=1",
        "https://cf-files.nexus-cdn.com/123/456/Mod.7z?md5=abc&user_id=7",
        "https://nexus-cdn.com/123/Mod.7z",
        "https://users.nexusmods.com/123/Mod.7z",
        "https://CF-FILES.NEXUS-CDN.COM/123/Mod.7z",
    ] {
        assert_eq!(
            NexusDownloadUrl::parse(value).unwrap().as_str(),
            value,
            "{value}"
        );
    }
}
