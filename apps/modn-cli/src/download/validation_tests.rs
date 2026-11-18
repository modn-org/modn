//! Transport classification, link validation and consent-text tests.

use modn_core::acquisition::GithubReleaseAsset;
use std::io::{self, Cursor, Read};

const EXPECTED: &str = "https://github.com/fixture/project/releases/download/v1/example.zip";
const METADATA: &str = r#"{
    "tag_name": "v1",
    "html_url": "https://github.com/fixture/project/releases/tag/v1",
    "assets": [{"name": "example.zip", "browser_download_url": "https://github.com/fixture/project/releases/download/v1/example.zip"}]
}"#;

/// A body whose first read fails with the wrapped timeout ureq produces.
struct WrappedTimeoutBody {
    failures: usize,
    body: &'static [u8],
    offset: usize,
}

impl Read for WrappedTimeoutBody {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.failures > 0 {
            self.failures -= 1;
            return Err(ureq::Error::Timeout(ureq::Timeout::Global).into_io());
        }
        let start = self.offset.min(self.body.len());
        let end = (start + buffer.len()).min(self.body.len());
        buffer[..end - start].copy_from_slice(&self.body[start..end]);
        self.offset = end;
        Ok(end - start)
    }
}

fn selector() -> GithubReleaseAsset {
    GithubReleaseAsset::new(
        "fixture/project".to_owned(),
        "v1".to_owned(),
        "example.zip".to_owned(),
    )
}

#[test]
fn a_body_read_with_a_wrapped_ureq_timeout_is_retried_once() {
    let calls = std::cell::Cell::new(0_u32);
    let resolved = super::fallback::resolve_metadata(&selector(), |_url| {
        calls.set(calls.get() + 1);
        if calls.get() == 1 {
            Ok(Box::new(WrappedTimeoutBody {
                failures: 1,
                body: METADATA.as_bytes(),
                offset: 0,
            }) as Box<dyn Read>)
        } else {
            Ok(Box::new(Cursor::new(METADATA.as_bytes())) as Box<dyn Read>)
        }
    })
    .expect("the retried wrapped timeout resolves the asset");
    assert_eq!(resolved, EXPECTED);
    assert_eq!(calls.get(), 2, "the wrapped timeout was not retried once");
}

#[test]
fn an_unrelated_other_io_error_is_not_retried() {
    let calls = std::cell::Cell::new(0_u32);
    let error = super::fallback::resolve_metadata(&selector(), |_url| {
        calls.set(calls.get() + 1);
        Ok(Box::new(UnrelatedBody) as Box<dyn Read>)
    })
    .expect_err("an unrelated body error refuses the source");
    assert_eq!(calls.get(), 1, "an unrelated body error was retried");
    assert!(
        format!("{error:#}").contains("after 1 attempt(s)"),
        "{error:#}"
    );
}

struct UnrelatedBody;

impl Read for UnrelatedBody {
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("unrelated decode failure"))
    }
}

#[test]
fn a_successful_body_still_resolves() {
    let calls = std::cell::Cell::new(0_u32);
    let resolved = super::fallback::resolve_metadata(&selector(), |_url| {
        calls.set(calls.get() + 1);
        Ok(Box::new(Cursor::new(METADATA.as_bytes())) as Box<dyn Read>)
    })
    .expect("a complete body resolves the asset");
    assert_eq!(resolved, EXPECTED);
    assert_eq!(calls.get(), 1);
}

fn link(mod_id: u64, file_id: u64) -> crate::nxm::NxmLink {
    crate::nxm::parse(
        &format!(
            "nxm://cyberpunk2077/mods/{mod_id}/files/{file_id}?key=fixturekey&expires=9999999999"
        ),
        1,
    )
    .expect("fixture nxm link")
}

#[test]
fn validate_links_accepts_a_secondary_nexus_source() {
    let temporary = tempfile::tempdir().expect("fixture");
    let root = temporary.path().join("manager");
    std::fs::create_dir_all(&root).expect("manager directory");
    let package = super::tests::package_with_sources(
        &root,
        &"0".repeat(64),
        serde_json::json!([
            {"kind": "https", "url": "https://fixture.invalid/example.zip"},
            {
                "kind": "nexus",
                "game_domain": "cyberpunk2077",
                "mod_id": 6945,
                "file_id": 222222,
                "file_name": "Second.zip"
            }
        ]),
    );
    super::validate_links(&[link(6945, 222222)], &[&package])
        .expect("a link for the secondary Nexus source is accepted");
    let rejected = super::validate_links(&[link(6945, 333333)], &[&package])
        .expect_err("a link that matches no Nexus source is rejected");
    assert!(
        format!("{rejected:#}").contains("333333"),
        "the rejection does not name the unrelated file: {rejected:#}"
    );
}

#[test]
fn the_download_consent_line_names_the_first_fetchable_source() {
    let sources: Vec<modn_core::buckets::PackageSource> =
        serde_json::from_value(serde_json::json!([
            {"kind": "manual", "filename": "Example 1.0.zip"},
            {"kind": "https", "url": "https://example.invalid/example.zip"}
        ]))
        .expect("sources");
    let line = crate::dependency_plan::download_line(
        "community/test-game/example",
        "1.0",
        &sources,
        &sources[0],
    );
    assert!(
        line.contains("https://example.invalid/example.zip"),
        "{line}"
    );
    assert!(!line.contains("manually"), "{line}");
}
