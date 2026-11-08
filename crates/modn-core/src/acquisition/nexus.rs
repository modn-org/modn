//! Network-free Nexus Mods download provider.
//!
//! The provider builds the credential-free cache URL, the download-link API
//! request for a pinned Nexus file and validates the CDN URL in the API
//! response. Callers supply HTTP transport, so this module needs no network
//! client, and secrets appear only in redacting types.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::fmt::{self, Write as _};
use std::io::Read;

/// A Nexus Mods game domain, such as `cyberpunk2077`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NexusGameDomain(String);

impl NexusGameDomain {
    /// Validate a game domain: 1 to 64 lowercase letters, digits and inner hyphens.
    pub fn new(value: &str) -> Result<Self> {
        let bytes = value.as_bytes();
        let edge = |byte: &u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
        if !(1..=64).contains(&bytes.len())
            || !bytes.iter().all(|byte| edge(byte) || *byte == b'-')
            || !bytes.first().is_some_and(edge)
            || !bytes.last().is_some_and(edge)
        {
            bail!("invalid Nexus game domain");
        }
        Ok(Self(value.to_owned()))
    }

    /// The domain as it appears in Nexus URLs.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A pinned file in a Nexus Mods game domain.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NexusFile {
    game_domain: NexusGameDomain,
    mod_id: u64,
    file_id: u64,
    file_name: String,
}

impl NexusFile {
    /// Validate a pinned Nexus file selector.
    pub fn new(game_domain: &str, mod_id: u64, file_id: u64, file_name: &str) -> Result<Self> {
        Ok(Self {
            game_domain: NexusGameDomain::new(game_domain)?,
            mod_id: validate_id(mod_id, "mod")?,
            file_id: validate_id(file_id, "file")?,
            file_name: validate_file_name(file_name)?,
        })
    }

    /// The credential-free cache key URL for this file.
    #[must_use]
    pub fn cache_url(&self) -> String {
        format!(
            "nexus://{}/mods/{}/files/{}/{}",
            self.game_domain.as_str(),
            self.mod_id,
            self.file_id,
            self.file_name
        )
    }

    /// Build the Nexus download-link request.
    ///
    /// A free Nexus account passes the `key` and `expires` values from an nxm
    /// link as `grant`. The result carries secrets; do not log it.
    #[must_use]
    pub fn download_link_request(
        &self,
        api_key: &NexusApiKey,
        grant: Option<&NexusDownloadGrant>,
    ) -> NexusRequest {
        let mut url = format!(
            "https://api.nexusmods.com/v1/games/{}/mods/{}/files/{}/download_link.json",
            self.game_domain.as_str(),
            self.mod_id,
            self.file_id
        );
        if let Some(grant) = grant {
            url.push_str("?key=");
            url.push_str(&percent_encode(grant.expose_secret()));
            url.push_str("&expires=");
            url.push_str(&grant.expires.to_string());
        }
        NexusRequest {
            url,
            headers: vec![
                ("apikey".to_owned(), api_key.expose_secret().to_owned()),
                ("Application-Name".to_owned(), "Modn".to_owned()),
                (
                    "Application-Version".to_owned(),
                    env!("CARGO_PKG_VERSION").to_owned(),
                ),
            ],
        }
    }

    /// Resolve the file to a validated CDN download URL.
    ///
    /// `fetch` performs the HTTP request and returns its status and body. The
    /// body is limited to 1 MiB. Errors never include the API key or grant.
    pub fn resolve<R: Read>(
        &self,
        api_key: &NexusApiKey,
        grant: Option<&NexusDownloadGrant>,
        fetch: impl FnOnce(&NexusRequest) -> Result<(u16, R)>,
    ) -> Result<NexusDownloadUrl> {
        let request = self.download_link_request(api_key, grant);
        let (status, body) = fetch(&request)?;
        match status {
            200 => {}
            400 => bail!("Nexus rejected the nxm link as invalid or expired"),
            401 => bail!("Nexus rejected the API key; check it and try again"),
            403 => bail!(
                "Nexus downloads need Premium, or a free account with the --nxm link from the Nexus website"
            ),
            404 => bail!("Nexus file not found"),
            410 => bail!("Nexus download link expired; get a new nxm link"),
            429 => bail!("Nexus rate limit reached; try again later"),
            status => bail!("Nexus download link request failed with HTTP status {status}"),
        }
        let mut bytes = Vec::new();
        body.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 1024 * 1024 {
            bail!("Nexus download link response exceeds the 1 MiB size limit");
        }
        let links: Vec<DownloadLink> =
            serde_json::from_slice(&bytes).context("invalid Nexus download link response")?;
        let link = links
            .into_iter()
            .next()
            .context("Nexus returned no download links")?;
        NexusDownloadUrl::parse(&link.uri)
    }
}

#[derive(Deserialize)]
struct DownloadLink {
    #[serde(rename = "URI")]
    uri: String,
}

#[derive(Clone)]
struct Secret(String);

impl Secret {
    fn new(value: &str, label: &str) -> Result<Self> {
        if value.is_empty() || value.chars().any(char::is_control) {
            bail!("{label} must be non-empty and contain no control characters");
        }
        Ok(Self(value.to_owned()))
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// A Nexus Mods API key. `Debug` prints `<redacted>`.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct NexusApiKey {
    secret: Secret,
}

impl NexusApiKey {
    /// Validate an API key: non-empty and free of control characters.
    pub fn new(value: &str) -> Result<Self> {
        Ok(Self {
            secret: Secret::new(value, "Nexus API key")?,
        })
    }

    /// Expose the secret to build a request. Never print the result.
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        self.secret.expose()
    }
}

/// The `key` and `expires` values from an nxm link, needed by free accounts.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct NexusDownloadGrant {
    key: Secret,
    expires: u64,
}

impl NexusDownloadGrant {
    /// Validate a download grant. `expires` is the nxm link expiry value.
    pub fn new(key: &str, expires: u64) -> Result<Self> {
        Ok(Self {
            key: Secret::new(key, "Nexus nxm key")?,
            expires,
        })
    }

    /// Expose the grant secret to build a request. Never print the result.
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        self.key.expose()
    }
}

/// One Nexus Mods download-link API request.
///
/// Secret-bearing: `apikey` holds the API key and the query holds the nxm
/// grant values. Send it only to `api.nexusmods.com`, never log it, and never
/// forward its headers to a CDN.
#[non_exhaustive]
pub struct NexusRequest {
    url: String,
    headers: Vec<(String, String)>,
}

impl NexusRequest {
    /// The request URL. With a grant, the query holds secret values.
    #[must_use]
    pub fn url_for_transport(&self) -> &str {
        &self.url
    }

    /// Request headers, including the secret API key in `apikey`.
    #[must_use]
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }
}

impl fmt::Debug for NexusRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let headers: Vec<(&str, &str)> = self
            .headers
            .iter()
            .map(|(name, value)| {
                let value = if name.eq_ignore_ascii_case("apikey") {
                    "<redacted>"
                } else {
                    value.as_str()
                };
                (name.as_str(), value)
            })
            .collect();
        f.debug_struct("NexusRequest")
            .field("url", &redact_grant_query(&self.url))
            .field("headers", &headers)
            .finish()
    }
}

/// A validated Nexus CDN download URL.
///
/// The URL carries a signed query string, which `Debug` hides. Send the URL
/// only to the CDN, and do not attach the Nexus `apikey` header to that
/// request: the API key belongs only to `api.nexusmods.com`.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct NexusDownloadUrl(String);

impl NexusDownloadUrl {
    fn parse(value: &str) -> Result<Self> {
        let Some(rest) = value.strip_prefix("https://") else {
            bail!("Nexus CDN URL must use https");
        };
        if value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '@' | '#' | '\\'))
        {
            bail!("Nexus CDN URL contains a forbidden character");
        }
        let (authority, path) = rest.split_once('/').context("Nexus CDN URL has no path")?;
        if authority.is_empty() {
            bail!("Nexus CDN URL has no host");
        }
        if path.is_empty() {
            bail!("Nexus CDN URL has no path");
        }
        if authority.contains(':') {
            bail!("Nexus CDN URL must not include a port");
        }
        let host = authority.to_ascii_lowercase();
        let label_ok = |label: &str| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        };
        if !host.split('.').all(label_ok) {
            bail!("Nexus CDN URL has an invalid host");
        }
        if !(host == "nexus-cdn.com"
            || host.ends_with(".nexus-cdn.com")
            || host == "nexusmods.com"
            || host.ends_with(".nexusmods.com"))
        {
            bail!("Nexus CDN URL host is not a Nexus host");
        }
        Ok(Self(value.to_owned()))
    }

    /// The validated URL, including its secret query string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for NexusDownloadUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("NexusDownloadUrl")
            .field(&redact_query(&self.0))
            .finish()
    }
}

fn validate_id(value: u64, label: &str) -> Result<u64> {
    if (1..=9_999_999_999).contains(&value) {
        Ok(value)
    } else {
        bail!("Nexus {label} ID must be between 1 and 9999999999");
    }
}

fn validate_file_name(value: &str) -> Result<String> {
    if value.is_empty()
        || value.chars().count() > 255
        || value == "."
        || value == ".."
        || value.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | '#' | '?' | '"' | '<' | '>' | '|' | ':')
        })
    {
        bail!("invalid Nexus file name");
    }
    Ok(value.to_owned())
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_') {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

fn redact_query(url: &str) -> String {
    match url.split_once('?') {
        Some((base, _)) => format!("{base}?<redacted>"),
        None => url.to_owned(),
    }
}

fn redact_grant_query(url: &str) -> String {
    let Some((base, query)) = url.split_once('?') else {
        return url.to_owned();
    };
    let query = query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((name, _))
                if name.eq_ignore_ascii_case("key") || name.eq_ignore_ascii_case("expires") =>
            {
                format!("{name}=<redacted>")
            }
            _ => pair.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("&");
    format!("{base}?{query}")
}

#[cfg(test)]
mod tests;
