//! GitHub release list parsing and selection.

use super::glob::glob_matches;
use super::version::github_tag_version;
use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
}

/// The selected upstream release and asset of a GitHub release check.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SelectedGithubRelease {
    /// The exact release tag, such as `v1.2.0`.
    pub tag: String,
    /// The release tag with one leading `v` removed.
    pub version: String,
    /// The one asset whose name matches the pattern.
    pub asset: String,
}

/// Select the newest eligible release and its single matching asset.
///
/// GitHub returns releases newest first. A draft never counts. A prerelease
/// counts only when `prerelease` is true. Exactly one asset must match
/// `asset_pattern`, where `*` and `?` are wildcards.
///
/// # Errors
///
/// Returns an error for malformed JSON, no eligible release, or zero or
/// several matching assets.
pub fn select_github_release(
    bytes: &[u8],
    prerelease: bool,
    asset_pattern: &str,
) -> Result<SelectedGithubRelease> {
    let releases: Vec<Release> =
        serde_json::from_slice(bytes).context("invalid GitHub release list")?;
    let release = releases
        .iter()
        .find(|release| !release.draft && (prerelease || !release.prerelease))
        .context("GitHub repository has no non-draft release")?;
    let mut matches = release
        .assets
        .iter()
        .filter(|asset| glob_matches(asset_pattern, &asset.name));
    let asset = matches.next().context(format!(
        "no asset of GitHub release {} matches {asset_pattern}",
        release.tag_name
    ))?;
    if matches.next().is_some() {
        bail!(
            "several assets of GitHub release {} match {asset_pattern}; make the pattern exact",
            release.tag_name
        );
    }
    Ok(SelectedGithubRelease {
        tag: release.tag_name.clone(),
        version: github_tag_version(&release.tag_name),
        asset: asset.name.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::select_github_release;

    fn releases(value: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&value).expect("fixture JSON")
    }

    #[test]
    fn checkver_github_release_selects_the_newest_eligible_release() {
        let bytes = releases(serde_json::json!([
            {"tag_name": "v2.0.0", "draft": true,
             "assets": [{"name": "Mod-2.0.0.zip"}]},
            {"tag_name": "v1.2.0", "prerelease": true,
             "assets": [{"name": "Mod-1.2.0.zip"}]},
            {"tag_name": "v1.1.0",
             "assets": [{"name": "Mod-1.1.0.zip"}]}
        ]));
        let selected = select_github_release(&bytes, false, "Mod-*.zip").expect("release");
        assert_eq!(selected.tag, "v1.1.0");
        assert_eq!(selected.version, "1.1.0");
        assert_eq!(selected.asset, "Mod-1.1.0.zip");
    }

    #[test]
    fn checkver_github_release_honours_the_prerelease_flag() {
        let bytes = releases(serde_json::json!([
            {"tag_name": "v2.0.0-beta.1", "prerelease": true,
             "assets": [{"name": "Mod-beta.zip"}]},
            {"tag_name": "v1.0.0", "assets": [{"name": "Mod-1.0.0.zip"}]}
        ]));
        assert_eq!(
            select_github_release(&bytes, false, "Mod-*.zip")
                .expect("stable")
                .version,
            "1.0.0"
        );
        assert_eq!(
            select_github_release(&bytes, true, "Mod-*.zip")
                .expect("prerelease")
                .version,
            "2.0.0-beta.1"
        );
    }

    #[test]
    fn checkver_github_release_requires_exactly_one_asset() {
        let none = releases(serde_json::json!([
            {"tag_name": "v1", "assets": [{"name": "other.zip"}]}
        ]));
        let error = format!(
            "{:#}",
            select_github_release(&none, false, "Mod-*.zip").unwrap_err()
        );
        assert!(error.contains("no asset"), "{error}");

        let several = releases(serde_json::json!([
            {"tag_name": "v1",
             "assets": [{"name": "Mod-a.zip"}, {"name": "Mod-b.zip"}]}
        ]));
        let error = format!(
            "{:#}",
            select_github_release(&several, false, "Mod-*.zip").unwrap_err()
        );
        assert!(error.contains("several assets"), "{error}");

        let exact = releases(serde_json::json!([
            {"tag_name": "v1", "assets": [{"name": "Mod-a.zip"}]}
        ]));
        assert!(select_github_release(&exact, false, "Mod-a.zip").is_ok());
    }

    #[test]
    fn checkver_github_release_refuses_a_list_without_a_release() {
        let bytes = releases(serde_json::json!([
            {"tag_name": "v1", "draft": true, "assets": [{"name": "Mod.zip"}]}
        ]));
        let error = format!(
            "{:#}",
            select_github_release(&bytes, false, "Mod.zip").unwrap_err()
        );
        assert!(error.contains("no non-draft release"), "{error}");
    }
}
