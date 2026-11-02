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
    pub archive_format: ArchiveFormat,
    pub sha256: String,
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
    Manual { filename: String },
    Https { url: String },
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
        if self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            bail!("package sha256 must be 64 lowercase hex characters");
        }
        match &self.source {
            PackageSource::Manual { filename } => {
                let extension = match self.archive_format {
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
            PackageSource::Https { url } if !valid_https(url) => {
                bail!("download URL must be HTTPS")
            }
            _ => {}
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

    fn manifest(extra: serde_json::Value) -> anyhow::Result<()> {
        let mut value = serde_json::json!({
            "schema_version": 1, "id": "top", "game": "test-game", "name": "Top",
            "version": "1.0", "archive_format": "zip", "sha256": "0".repeat(64),
            "source": {"kind": "manual", "filename": "top.zip"},
            "dependencies": ["base", "xl"]
        });
        if let (Some(value), serde_json::Value::Object(extra)) = (value.as_object_mut(), extra) {
            value.extend(extra);
        }
        let manifest: super::PackageManifest = serde_json::from_value(value)?;
        manifest.validate("test-game", "top")
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
}
