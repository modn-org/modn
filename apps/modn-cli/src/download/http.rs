//! HTTP fetching for artifact sources.

use anyhow::{bail, Result};
use std::io::Read;
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const READ_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(60);

#[cfg(test)]
use super::fixture_response;

pub(super) fn fetch_metadata(url: &str) -> Result<Box<dyn Read>> {
    #[cfg(test)]
    if let Some(response) = fixture_response(url) {
        return response;
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(0)
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_per_read(Some(READ_INACTIVITY_TIMEOUT))
        .user_agent(concat!("modn/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let response = agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .call()?;
    if response.status() != 200 {
        bail!("GitHub metadata request returned {}", response.status());
    }
    Ok(Box::new(response.into_body().into_reader()))
}

pub(crate) fn fetch(url: &str) -> Result<Box<dyn Read>> {
    Ok(fetch_measured(url)?.0)
}

pub(super) fn fetch_measured(url: &str) -> Result<(Box<dyn Read>, Option<u64>)> {
    #[cfg(test)]
    if let Some(response) = fixture_response(url) {
        return Ok((response?, None));
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_per_read(Some(READ_INACTIVITY_TIMEOUT))
        .user_agent(concat!("modn/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let response = agent.get(url).call()?;
    let total = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok());
    Ok((Box::new(response.into_body().into_reader()), total))
}
