use modn_core::acquisition::{FetchError, SourceBody, SourcePolicy, SourceTransport};
use std::io::Read;

#[cfg(test)]
use super::fixture_response;

fn agent(policy: &SourcePolicy, redirects: bool) -> ureq::Agent {
    let mut builder = ureq::Agent::config_builder()
        .https_only(true)
        .timeout_connect(Some(policy.connect_timeout()))
        .timeout_per_read(Some(policy.idle_timeout()))
        .user_agent(concat!("modn/", env!("CARGO_PKG_VERSION")));
    if !redirects {
        builder = builder.max_redirects(0);
    }
    builder.build().into()
}

fn classify(error: ureq::Error) -> FetchError {
    match error {
        ureq::Error::StatusCode(status) => FetchError::Http(status),
        ureq::Error::Timeout(_) => FetchError::Timeout,
        other => FetchError::Network(other.to_string()),
    }
}

pub(super) fn fetch_metadata(url: &str) -> anyhow::Result<Box<dyn Read>> {
    #[cfg(test)]
    if let Some(failure) = super::take_metadata_failure(url) {
        return Err(failure.into());
    }
    #[cfg(test)]
    if let Some(response) = fixture_response(url) {
        return response;
    }
    let response = agent(&SourcePolicy::default(), false)
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .call()
        .map_err(classify)?;
    if response.status() != 200 {
        anyhow::bail!("GitHub metadata request returned {}", response.status());
    }
    Ok(Box::new(response.into_body().into_reader()))
}

pub(crate) fn fetch(url: &str) -> anyhow::Result<Box<dyn Read>> {
    Ok(fetch_measured(url)?.0)
}

pub(super) fn fetch_measured(url: &str) -> anyhow::Result<(Box<dyn Read>, Option<u64>)> {
    let body = HttpTransport.open(url, &SourcePolicy::default())?;
    Ok((body.reader, body.total))
}

pub(super) struct HttpTransport;

impl SourceTransport for HttpTransport {
    fn open(&self, url: &str, policy: &SourcePolicy) -> Result<SourceBody, FetchError> {
        #[cfg(test)]
        if let Some(response) = fixture_response(url) {
            return response
                .map(|reader| SourceBody::new(reader, None))
                .map_err(|error| FetchError::Network(format!("{error:#}")));
        }
        let response = agent(policy, true).get(url).call().map_err(classify)?;
        let total = response
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok());
        Ok(SourceBody::new(
            Box::new(response.into_body().into_reader()),
            total,
        ))
    }
}
