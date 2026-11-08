use crate::nxm::NxmLink;
use anyhow::{bail, Context, Result};
use modn_core::acquisition::{NexusApiKey, NexusDownloadUrl, NexusFile, NexusRequest};
use std::io::Read;
use std::time::Duration;

pub(super) const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const READ_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_CDN_REDIRECTS: u32 = 5;

pub(super) fn validate_links(links: &[NxmLink], packages: &[(&str, u64, u64)]) -> Result<()> {
    for link in links {
        let (domain, mod_id, file_id) = (link.file.0.as_str(), link.file.1, link.file.2);
        if !packages
            .iter()
            .any(|(package_domain, package_mod, package_file)| {
                *package_domain == domain && *package_mod == mod_id && *package_file == file_id
            })
        {
            bail!(
                "nxm link for mod {mod_id} file {file_id} does not match a package being installed or updated"
            );
        }
    }
    Ok(())
}

pub(super) fn matching_link<'a>(
    links: &'a [NxmLink],
    game_domain: &str,
    mod_id: u64,
    file_id: u64,
) -> Option<&'a NxmLink> {
    links.iter().find(|link| {
        link.file.0.as_str() == game_domain && link.file.1 == mod_id && link.file.2 == file_id
    })
}

pub(super) fn resolve(
    file: &NexusFile,
    api_key: &NexusApiKey,
    link: Option<&NxmLink>,
) -> Result<NexusDownloadUrl> {
    file.resolve(api_key, link.map(|link| &link.grant), fetch_api)
}

fn fetch_api(request: &NexusRequest) -> Result<(u16, Box<dyn Read>)> {
    let url = request.url_for_transport();
    if !url.starts_with("https://api.nexusmods.com/") {
        bail!("Nexus API request URL is not api.nexusmods.com");
    }
    #[cfg(test)]
    if let Some(response) = fixture_response(url, request.headers()) {
        let response = response?;
        if (300..400).contains(&response.status) {
            bail!("Nexus API request must not redirect");
        }
        return Ok((
            response.status,
            Box::new(std::io::Cursor::new(response.body)),
        ));
    }
    let mut call = agent().get(url);
    for (name, value) in request.headers() {
        call = call.header(name.as_str(), value.as_str());
    }
    let response = call.call().map_err(api_error)?;
    let status = response.status().as_u16();
    if (300..400).contains(&status) {
        bail!("Nexus API request must not redirect");
    }
    Ok((status, Box::new(response.into_body().into_reader())))
}

pub(super) fn open(url: &NexusDownloadUrl) -> Result<Box<dyn Read>> {
    let mut current = url.as_str().to_owned();
    let mut followed = 0;
    loop {
        let (status, location, body) = request_cdn(&current)?;
        if (300..400).contains(&status) {
            if followed == MAX_CDN_REDIRECTS {
                bail!("Nexus CDN redirected more than {MAX_CDN_REDIRECTS} times");
            }
            let location = location.context("Nexus CDN redirect has no Location header")?;
            validate_location(&location)?;
            current = location;
            followed += 1;
            continue;
        }
        if status != 200 {
            bail!("Nexus CDN download failed with HTTP status {status}");
        }
        return Ok(body);
    }
}

fn request_cdn(url: &str) -> Result<(u16, Option<String>, Box<dyn Read>)> {
    #[cfg(test)]
    if let Some(response) = fixture_response(url, &[]) {
        let response = response?;
        let location = response
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("location"))
            .map(|(_, value)| value.clone());
        return Ok((
            response.status,
            location,
            Box::new(std::io::Cursor::new(response.body)),
        ));
    }
    let response = agent().get(url).call().map_err(cdn_error)?;
    let status = response.status().as_u16();
    let location = response
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    Ok((
        status,
        location,
        Box::new(response.into_body().into_reader()),
    ))
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(0)
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_per_read(Some(READ_INACTIVITY_TIMEOUT))
        .user_agent(concat!("modn/", env!("CARGO_PKG_VERSION")))
        .http_status_as_error(false)
        .build()
        .into()
}

fn api_error(error: ureq::Error) -> anyhow::Error {
    match error {
        ureq::Error::Io(error) => anyhow::anyhow!("Nexus API request failed: {}", error.kind()),
        ureq::Error::Timeout(_) => anyhow::anyhow!("Nexus API request timed out"),
        ureq::Error::HostNotFound => anyhow::anyhow!("Nexus API host not found"),
        ureq::Error::StatusCode(status) => {
            anyhow::anyhow!("Nexus API request failed with HTTP status {status}")
        }
        _ => anyhow::anyhow!("Nexus API request failed"),
    }
}

fn cdn_error(error: ureq::Error) -> anyhow::Error {
    match error {
        ureq::Error::Io(error) => anyhow::anyhow!("Nexus CDN request failed: {}", error.kind()),
        ureq::Error::Timeout(_) => anyhow::anyhow!("Nexus CDN request timed out"),
        ureq::Error::HostNotFound => anyhow::anyhow!("Nexus CDN host not found"),
        ureq::Error::StatusCode(status) => {
            anyhow::anyhow!("Nexus CDN download failed with HTTP status {status}")
        }
        _ => anyhow::anyhow!("Nexus CDN request failed"),
    }
}

fn validate_location(value: &str) -> Result<()> {
    let Some(rest) = value.strip_prefix("https://") else {
        bail!("Nexus CDN redirect must use https");
    };
    if value.chars().any(|character| {
        character.is_whitespace() || character.is_control() || matches!(character, '@' | '#' | '\\')
    }) {
        bail!("Nexus CDN redirect URL contains a forbidden character");
    }
    let (authority, path) = rest
        .split_once('/')
        .context("Nexus CDN redirect has no path")?;
    if authority.is_empty() {
        bail!("Nexus CDN redirect has no host");
    }
    if path.is_empty() {
        bail!("Nexus CDN redirect has no path");
    }
    if authority.contains(':') {
        bail!("Nexus CDN redirect must not include a port");
    }
    let host = authority.to_ascii_lowercase();
    let label_ok = |label: &str| {
        !label.is_empty()
            && label
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    };
    if !host.split('.').all(label_ok) {
        bail!("Nexus CDN redirect has an invalid host");
    }
    if !(host == "nexus-cdn.com"
        || host.ends_with(".nexus-cdn.com")
        || host == "nexusmods.com"
        || host.ends_with(".nexusmods.com"))
    {
        bail!("Nexus CDN redirect host is not a Nexus host");
    }
    Ok(())
}

#[cfg(test)]
#[derive(Clone)]
pub(crate) struct FixtureResponse {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

#[cfg(test)]
impl FixtureResponse {
    pub(crate) fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    pub(crate) fn status(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub(crate) fn redirect(location: &str) -> Self {
        Self {
            status: 302,
            headers: vec![("location".to_owned(), location.to_owned())],
            body: Vec::new(),
        }
    }
}

#[cfg(test)]
#[derive(Clone)]
pub(crate) struct RecordedRequest {
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
}

#[cfg(test)]
thread_local! {
    static TEST_NEXUS_RESPONSES: std::cell::RefCell<Option<std::collections::BTreeMap<String, FixtureResponse>>> =
        const { std::cell::RefCell::new(None) };
    static TEST_NEXUS_REQUESTS: std::cell::RefCell<Vec<RecordedRequest>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub(crate) fn set_responses(responses: std::collections::BTreeMap<String, FixtureResponse>) {
    TEST_NEXUS_RESPONSES.with(|cell| cell.borrow_mut().replace(responses));
    TEST_NEXUS_REQUESTS.with(|cell| cell.borrow_mut().clear());
}

#[cfg(test)]
pub(crate) fn requests() -> Vec<RecordedRequest> {
    TEST_NEXUS_REQUESTS.with(|cell| cell.borrow().clone())
}

#[cfg(test)]
fn fixture_response(url: &str, headers: &[(String, String)]) -> Option<Result<FixtureResponse>> {
    TEST_NEXUS_RESPONSES.with(|cell| {
        cell.borrow().as_ref().map(|responses| {
            TEST_NEXUS_REQUESTS.with(|requests| {
                requests.borrow_mut().push(RecordedRequest {
                    url: url.to_owned(),
                    headers: headers.to_vec(),
                });
            });
            responses
                .get(url)
                .cloned()
                .with_context(|| format!("unexpected Nexus fixture request: {url}"))
        })
    })
}
