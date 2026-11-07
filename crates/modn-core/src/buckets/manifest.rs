use super::version::VersionReq;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

/// One immutable package version, independent of installer policy.
#[expect(
    missing_docs,
    reason = "Serialized schema fields directly specify their purpose"
)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PackageManifest {
    pub schema_version: u32,
    pub id: String,
    pub game: String,
    pub name: String,
    pub version: String,
    /// Required for every source except `git`, whose archive is always ZIP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_format: Option<ArchiveFormat>,
    /// Archive digest. Required except for `git`, whose commit is the pin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub source: PackageSource,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// Game-relative tool executables the archive installs. A declaration
    /// lets Modn name them in its confirmation prompt; it never approves them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub executables: Vec<String>,
    /// Version requirement for each listed dependency that needs one, such
    /// as `{"red4ext": ">=1.31.0"}`. The bucket's release must satisfy it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependency_versions: BTreeMap<String, String>,
    /// Mods in the same bucket and game that must not be installed together
    /// with this one, in either order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<String>,
    /// True when the archive places a native library that the game process
    /// loads, such as `winmm.dll` beside the game executable.
    #[serde(default)]
    pub native_injector: bool,
}

/// Format of the original, unextracted download.
#[expect(
    missing_docs,
    reason = "Serialized schema fields directly specify their purpose"
)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum ArchiveFormat {
    Zip,
    #[serde(rename = "7z")]
    SevenZ,
}

/// Metadata only: no URLs are fetched by the bucket foundation.
#[expect(
    missing_docs,
    reason = "Serialized schema fields directly specify their purpose"
)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
#[non_exhaustive]
pub enum PackageSource {
    Manual {
        filename: String,
    },
    Https {
        url: String,
    },
    #[serde(rename = "github-release")]
    GithubRelease {
        repository: String,
        tag: String,
        asset: String,
    },
    Git {
        repository: String,
        /// Full lowercase 40-character commit, the integrity pin.
        commit: String,
    },
}

pub(super) fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !value.ends_with('-')
}

const MAX_EXECUTABLES: usize = 16;
const MAX_CONFLICTS: usize = 32;

fn valid_tool_executable(path: &str) -> bool {
    path.len() <= 255
        && path.ends_with(".exe")
        && !path.starts_with('/')
        && !path.contains(['\\', ':'])
        && !path.chars().any(char::is_control)
        && path.split('/').any(|part| part == "tools")
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn valid_https(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or("");
    !host.is_empty()
        && !host.contains('@')
        && !host.contains('#')
        && !host.contains('?')
        && !url.chars().any(char::is_whitespace)
}

impl PackageManifest {
    pub(super) fn validate(&self, game: &str, id: &str) -> Result<()> {
        if self.schema_version != 1 {
            bail!("unsupported bucket package schema");
        }
        if !valid_id(&self.id) || !valid_id(&self.game) || self.id != id || self.game != game {
            bail!("bucket package identity disagrees with games/<game>/<id>.json path");
        }
        if self.name.trim().is_empty() || self.name.len() > 128 {
            bail!("invalid package name");
        }
        if self.version.is_empty()
            || self.version.len() > 64
            || !self
                .version
                .bytes()
                .all(|b| b.is_ascii_graphic() && b != b'/' && b != b'\\')
        {
            bail!("invalid package version");
        }
        match &self.source {
            PackageSource::Git { repository, commit } => {
                if self.sha256.is_some() {
                    bail!("git source packages must not declare sha256; the commit is the pin");
                }
                if self
                    .archive_format
                    .as_ref()
                    .is_some_and(|format| !matches!(format, ArchiveFormat::Zip))
                {
                    bail!("git source archive_format must be zip when present");
                }
                crate::acquisition::GitSource::new(repository.clone(), commit.clone())
                    .remote_url()?;
            }
            PackageSource::GithubRelease {
                repository,
                tag,
                asset,
            } => {
                let sha256 = self
                    .sha256
                    .as_deref()
                    .context("package sha256 is required for non-git sources")?;
                if sha256.len() != 64
                    || !sha256
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    bail!("package sha256 must be 64 lowercase hex characters");
                }
                crate::acquisition::GithubReleaseAsset::new(
                    repository.clone(),
                    tag.clone(),
                    asset.clone(),
                )
                .metadata_url()?;
            }
            PackageSource::Manual { filename } => {
                let sha256 = self
                    .sha256
                    .as_deref()
                    .context("package sha256 is required for non-git sources")?;
                if sha256.len() != 64
                    || !sha256
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    bail!("package sha256 must be 64 lowercase hex characters");
                }
                let format = self
                    .archive_format
                    .as_ref()
                    .context("archive_format is required for non-git sources")?;
                let extension = match format {
                    ArchiveFormat::Zip => ".zip",
                    ArchiveFormat::SevenZ => ".7z",
                };
                if filename.is_empty()
                    || filename.len() > 255
                    || filename == "."
                    || filename == ".."
                    || filename
                        .chars()
                        .any(|c| c.is_control() || c == '/' || c == '\\')
                    || !filename.to_ascii_lowercase().ends_with(extension)
                {
                    bail!("invalid manual archive filename");
                }
            }
            PackageSource::Https { url } => {
                let sha256 = self
                    .sha256
                    .as_deref()
                    .context("package sha256 is required for non-git sources")?;
                if sha256.len() != 64
                    || !sha256
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    bail!("package sha256 must be 64 lowercase hex characters");
                }
                self.archive_format
                    .as_ref()
                    .context("archive_format is required for non-git sources")?;
                if !valid_https(url) {
                    bail!("download URL must be HTTPS")
                }
            }
        }
        if self.homepage.as_ref().is_some_and(|url| !valid_https(url)) {
            bail!("homepage URL must be HTTPS");
        }
        let mut declared = HashSet::new();
        if self.executables.len() > MAX_EXECUTABLES
            || !self
                .executables
                .iter()
                .all(|path| valid_tool_executable(path) && declared.insert(path))
        {
            bail!("invalid or duplicate declared tool executable");
        }
        let mut unique = HashSet::new();
        for dependency in &self.dependencies {
            if !valid_id(dependency) || dependency == &self.id || !unique.insert(dependency) {
                bail!("invalid, self-referential or duplicate dependency");
            }
        }
        for (dependency, requirement) in &self.dependency_versions {
            if !unique.contains(dependency) {
                bail!("dependency_versions names {dependency}, which is not a dependency");
            }
            if requirement.len() > 64 {
                bail!("version requirement for {dependency} is too long");
            }
            VersionReq::parse(requirement)
                .with_context(|| format!("invalid version requirement for {dependency}"))?;
        }
        let mut conflicts = HashSet::new();
        if self.conflicts.len() > MAX_CONFLICTS {
            bail!("too many conflicts");
        }
        for conflict in &self.conflicts {
            if !valid_id(conflict)
                || conflict == &self.id
                || unique.contains(conflict)
                || !conflicts.insert(conflict)
            {
                bail!("invalid, self-referential, duplicate or depended-on conflict: {conflict}");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::valid_tool_executable;
    use crate::planning::approved_tool_executable;

    #[test]
    fn declarations_are_limited_to_approvable_tool_executables() {
        for path in ["engine/tools/scc.exe", "tools/a.exe", "x/tools/y/z.exe"] {
            assert!(valid_tool_executable(path), "{path}");
            let approval = [path.to_owned()];
            assert!(approved_tool_executable(path, path, &approval), "{path}");
        }
        for path in [
            "",
            "bin/x64/Cyberpunk2077.exe",
            "engine/tools/scc.dll",
            "engine/tools/scc.EXE",
            "/engine/tools/scc.exe",
            r"engine\tools\scc.exe",
            "C:/engine/tools/scc.exe",
            "engine/tools/../scc.exe",
            "engine/./tools/scc.exe",
            "engine//tools/scc.exe",
            "engine/tools/scc.bat",
        ] {
            assert!(!valid_tool_executable(path), "{path:?}");
        }
    }

    fn parse(extra: serde_json::Value) -> anyhow::Result<super::PackageManifest> {
        let mut value = serde_json::json!({
            "schema_version": 1, "id": "top", "game": "test-game", "name": "Top",
            "version": "1.0", "archive_format": "zip", "sha256": "0".repeat(64),
            "source": {"kind": "manual", "filename": "top.zip"},
            "dependencies": ["base", "xl"]
        });
        if let (Some(value), serde_json::Value::Object(extra)) = (value.as_object_mut(), extra) {
            value.extend(extra);
        }
        Ok(serde_json::from_value(value)?)
    }

    fn manifest(extra: serde_json::Value) -> anyhow::Result<()> {
        parse(extra)?.validate("test-game", "top")
    }

    #[test]
    fn native_injector_parses_true_false_and_absent() {
        assert!(
            !parse(serde_json::json!({}))
                .expect("absent")
                .native_injector
        );
        assert!(
            !parse(serde_json::json!({"native_injector": false}))
                .expect("false")
                .native_injector
        );
        assert!(
            parse(serde_json::json!({"native_injector": true}))
                .expect("true")
                .native_injector
        );
    }

    #[test]
    fn non_boolean_native_injector_is_rejected_as_malformed() {
        for value in [
            serde_json::json!("true"),
            serde_json::json!(1),
            serde_json::json!(null),
        ] {
            let error = format!(
                "{:#}",
                parse(serde_json::json!({"native_injector": value})).unwrap_err()
            );
            assert!(error.contains("boolean"), "{value}: {error}");
        }
    }

    #[test]
    fn version_requirements_and_conflicts_are_validated() {
        use serde_json::json;
        manifest(json!({})).expect("no new fields");
        manifest(json!({
            "dependency_versions": {"base": ">=1.31.0", "xl": ">=1.28.0, <2"},
            "conflicts": ["rival"]
        }))
        .expect("valid fields");
        for (extra, message) in [
            (
                json!({"dependency_versions": {"other": ">=1"}}),
                "not a dependency",
            ),
            (
                json!({"dependency_versions": {"base": "1.0"}}),
                "invalid version requirement",
            ),
            (json!({"conflicts": ["top"]}), "self-referential"),
            (json!({"conflicts": ["base"]}), "depended-on"),
            (json!({"conflicts": ["rival", "rival"]}), "duplicate"),
            (json!({"conflicts": ["Bad_ID"]}), "invalid"),
        ] {
            let error = format!("{:#}", manifest(extra.clone()).unwrap_err());
            assert!(error.contains(message), "{extra}: {error}");
        }
    }

    fn parse_git(extra: serde_json::Value) -> anyhow::Result<super::PackageManifest> {
        let mut value = serde_json::json!({
            "schema_version": 1, "id": "top", "game": "test-game", "name": "Top",
            "version": "1.0",
            "source": {"kind": "git", "repository": "owner/project", "commit": "a".repeat(40)}
        });
        if let (Some(value), serde_json::Value::Object(extra)) = (value.as_object_mut(), extra) {
            value.extend(extra);
        }
        Ok(serde_json::from_value(value)?)
    }

    fn git(extra: serde_json::Value) -> anyhow::Result<()> {
        parse_git(extra)?.validate("test-game", "top")
    }

    #[test]
    fn git_source_parses_with_a_full_commit_and_no_digest() {
        use serde_json::json;
        let parsed = parse_git(json!({})).expect("valid git source");
        assert!(parsed.sha256.is_none());
        assert!(parsed.archive_format.is_none());
        assert!(matches!(
            parsed.source,
            super::PackageSource::Git { repository, commit }
                if repository == "owner/project" && commit == "a".repeat(40)
        ));
        git(json!({"archive_format": "zip"})).expect("zip is allowed when present");
        for commit in ["abc", &"A".repeat(40), &"a".repeat(64)] {
            let error = format!(
                "{:#}",
                git(json!({
                    "source": {"kind": "git", "repository": "owner/project", "commit": commit}
                }))
                .unwrap_err()
            );
            assert!(error.contains("40 lowercase hex"), "{commit}: {error}");
        }
    }

    #[test]
    fn git_source_rejects_a_branch_name_an_unknown_field_and_a_digest() {
        use serde_json::json;
        let branch = format!(
            "{:#}",
            git(json!({
                "source": {"kind": "git", "repository": "owner/project", "commit": "main"}
            }))
            .unwrap_err()
        );
        assert!(branch.contains("40 lowercase hex"), "{branch}");

        let unknown = format!(
            "{:#}",
            parse_git(json!({
                "source": {"kind": "git", "repository": "owner/project",
                    "commit": "a".repeat(40), "tag": "v1"}
            }))
            .unwrap_err()
        );
        assert!(unknown.contains("unknown field"), "{unknown}");

        let digest = format!("{:#}", git(json!({"sha256": "0".repeat(64)})).unwrap_err());
        assert!(digest.contains("must not declare sha256"), "{digest}");
    }

    #[test]
    fn non_git_sources_still_require_sha256() {
        use serde_json::json;
        for source in [
            json!({"kind": "manual", "filename": "top.zip"}),
            json!({"kind": "https", "url": "https://example.invalid/top.zip"}),
            json!({"kind": "github-release", "repository": "owner/project",
                "tag": "v1", "asset": "top.zip"}),
        ] {
            let error = format!(
                "{:#}",
                manifest(json!({"sha256": null, "source": source})).unwrap_err()
            );
            assert!(error.contains("sha256 is required"), "{error}");
        }
    }
}
