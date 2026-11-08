use anyhow::{bail, Context, Result};
use modn_core::acquisition::{NexusDownloadGrant, NexusGameDomain};
use std::fmt;

const MAX_ID: u64 = 9_999_999_999;

/// A parsed Nexus "Mod Manager Download" link.
#[derive(Clone)]
pub(super) struct NxmLink {
    pub(super) file: (NexusGameDomain, u64, u64),
    pub(super) grant: NexusDownloadGrant,
}

impl fmt::Debug for NxmLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NxmLink { <redacted> }")
    }
}

/// Parse every `--nxm` value, naming the argument position on failure.
pub(super) fn parse_all(values: &[String], now: u64) -> Result<Vec<NxmLink>> {
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            parse(value, now).with_context(|| format!("invalid --nxm link {}", index + 1))
        })
        .collect()
}

/// Parse one nxm link whose `expires` value must be later than `now`.
pub(super) fn parse(value: &str, now: u64) -> Result<NxmLink> {
    if value
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
    {
        bail!("{} contains whitespace or control characters", hidden());
    }
    let Some(rest) = value.strip_prefix("nxm://") else {
        bail!("{} must start with nxm://", hidden());
    };
    if rest.contains('#') {
        bail!("{} must not contain a fragment", hidden());
    }
    let (authority, remainder) = rest.split_once('/').unwrap_or((rest, ""));
    if authority.contains('@') {
        bail!("{} must not contain user information", hidden());
    }
    if authority.contains(':') {
        bail!("{} must not contain a port", hidden());
    }
    let Ok(game_domain) = NexusGameDomain::new(authority) else {
        bail!("{} has an invalid game domain", hidden());
    };
    let (path, query) = remainder.split_once('?').unwrap_or((remainder, ""));
    let mut segments = path.split('/');
    let (Some("mods"), Some(mod_text), Some("files"), Some(file_text)) = (
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
    ) else {
        bail!(
            "{} must use the path /mods/<mod_id>/files/<file_id>",
            hidden()
        );
    };
    if segments.next().is_some() {
        bail!(
            "{} must use the path /mods/<mod_id>/files/<file_id>",
            hidden()
        );
    }
    let mod_id =
        parse_id(mod_text).with_context(|| format!("{} has an invalid mod ID", hidden()))?;
    let file_id =
        parse_id(file_text).with_context(|| format!("{} has an invalid file ID", hidden()))?;
    let label = format!("nxm link for mod {mod_id} file {file_id}");

    if query.is_empty() {
        bail!("{label} is missing the key and expires parameters");
    }
    let mut key = None;
    let mut expires = None;
    let mut user_id = None;
    for pair in query.split('&') {
        let Some((name, value)) = pair.split_once('=') else {
            bail!("{label} has an invalid query parameter");
        };
        match name {
            "key" => {
                if key.replace(value).is_some() {
                    bail!("{label} repeats the key parameter");
                }
            }
            "expires" => {
                let Some(seconds) = parse_number(value) else {
                    bail!("{label} has an invalid expires parameter");
                };
                if expires.replace(seconds).is_some() {
                    bail!("{label} repeats the expires parameter");
                }
            }
            "user_id" => {
                if parse_number(value).is_none() || user_id.replace(value).is_some() {
                    bail!("{label} has an invalid or repeated user_id parameter");
                }
            }
            _ => bail!("{label} has an unknown query parameter"),
        }
    }
    let Some(key) = key else {
        bail!("{label} is missing the key parameter");
    };
    let Some(expires) = expires else {
        bail!("{label} is missing the expires parameter");
    };
    if expires <= now {
        bail!("{label} has expired; get a new link from Nexus");
    }
    let grant = NexusDownloadGrant::new(key, expires)
        .with_context(|| format!("{label} has an invalid key"))?;
    Ok(NxmLink {
        file: (game_domain, mod_id, file_id),
        grant,
    })
}

fn hidden() -> &'static str {
    "the nxm link (contents hidden)"
}

fn parse_number(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn parse_id(value: &str) -> Option<u64> {
    parse_number(value).filter(|id| (1..=MAX_ID).contains(id))
}

#[cfg(test)]
mod tests;
