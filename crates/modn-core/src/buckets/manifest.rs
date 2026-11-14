use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

mod nexus;
mod validation;

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
    /// BLAKE3 archive pin, 64 lowercase hex characters. Required for every
    /// source except `git`, whose commit is the pin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blake3: Option<String>,
    /// Ordered artifact sources tried in turn for this release.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<PackageSource>,
    /// True when the author allows redistribution of the archive.
    #[serde(default)]
    pub redistributable: bool,
    /// Upstream SHA-256, when known. Provenance only; Modn never verifies it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_sha256: Option<String>,
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
    Nexus {
        game_domain: String,
        mod_id: u64,
        file_id: u64,
        file_name: String,
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

#[cfg(test)]
mod tests {
    use super::validation::valid_tool_executable;
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
            "schema_version": 2, "id": "top", "game": "test-game", "name": "Top",
            "version": "1.0", "archive_format": "zip", "blake3": "0".repeat(64),
            "sources": [{"kind": "manual", "filename": "top.zip"}],
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
            "schema_version": 2, "id": "top", "game": "test-game", "name": "Top",
            "version": "1.0",
            "sources": [{"kind": "git", "repository": "owner/project", "commit": "a".repeat(40)}]
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
        assert!(parsed.blake3.is_none());
        assert!(parsed.archive_format.is_none());
        assert!(matches!(
            parsed.primary_source(),
            super::PackageSource::Git { repository, commit }
                if repository == "owner/project" && *commit == "a".repeat(40)
        ));
        git(json!({"archive_format": "zip"})).expect("zip is allowed when present");
        for commit in ["abc", &"A".repeat(40), &"a".repeat(64)] {
            let error = format!(
                "{:#}",
                git(json!({
                    "sources": [{"kind": "git", "repository": "owner/project", "commit": commit}]
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
                "sources": [{"kind": "git", "repository": "owner/project", "commit": "main"}]
            }))
            .unwrap_err()
        );
        assert!(branch.contains("40 lowercase hex"), "{branch}");

        let unknown = format!(
            "{:#}",
            parse_git(json!({
                "sources": [{"kind": "git", "repository": "owner/project",
                    "commit": "a".repeat(40), "tag": "v1"}]
            }))
            .unwrap_err()
        );
        assert!(unknown.contains("unknown field"), "{unknown}");

        let digest = format!("{:#}", git(json!({"blake3": "0".repeat(64)})).unwrap_err());
        assert!(digest.contains("must not declare blake3"), "{digest}");
    }

    #[test]
    fn non_git_sources_still_require_blake3() {
        use serde_json::json;
        for source in [
            json!({"kind": "manual", "filename": "top.zip"}),
            json!({"kind": "https", "url": "https://example.invalid/top.zip"}),
            json!({"kind": "github-release", "repository": "owner/project",
                "tag": "v1", "asset": "top.zip"}),
        ] {
            let error = format!(
                "{:#}",
                manifest(json!({"blake3": null, "sources": [source]})).unwrap_err()
            );
            assert!(error.contains("blake3 is required"), "{error}");
        }
    }

    #[test]
    fn contract_v2_record_keeps_ordered_sources_and_optional_fields() {
        use serde_json::json;
        let parsed = parse(json!({
            "sources": [
                {"kind": "https", "url": "https://primary.invalid/top.zip"},
                {"kind": "https", "url": "https://mirror.invalid/top.zip"}
            ],
            "redistributable": true,
            "upstream_sha256": "b".repeat(64)
        }))
        .expect("valid v2 record");
        assert_eq!(parsed.schema_version, 2);
        assert_eq!(parsed.pin(), Some("0".repeat(64).as_str()));
        assert_eq!(
            parsed
                .sources
                .iter()
                .map(|source| match source {
                    super::PackageSource::Https { url } => url.as_str(),
                    _ => "other",
                })
                .collect::<Vec<_>>(),
            [
                "https://primary.invalid/top.zip",
                "https://mirror.invalid/top.zip"
            ]
        );
        assert!(parsed.redistributable);
        assert_eq!(parsed.upstream_sha256, Some("b".repeat(64)));
        let default = parse(json!({})).expect("redistributable defaults to false");
        assert!(!default.redistributable);
        assert!(default.upstream_sha256.is_none());
    }

    #[test]
    fn contract_v1_fields_are_refused() {
        use serde_json::json;
        let schema = format!("{:#}", manifest(json!({"schema_version": 1})).unwrap_err());
        assert!(schema.contains("schema_version 1"), "{schema}");
        assert!(schema.contains("contract v2"), "{schema}");
        for extra in [
            json!({"sha256": "0".repeat(64)}),
            json!({"source": {"kind": "manual", "filename": "top.zip"}}),
        ] {
            let error = format!("{:#}", parse(extra.clone()).unwrap_err());
            assert!(error.contains("unknown field"), "{extra}: {error}");
        }
        let boolean = format!(
            "{:#}",
            manifest(json!({"redistributable": "yes"})).unwrap_err()
        );
        assert!(boolean.contains("boolean"), "{boolean}");
        for extra in [
            json!({"blake3": "A".repeat(64)}),
            json!({"blake3": "0".repeat(63)}),
            json!({"upstream_sha256": "xyz"}),
        ] {
            let error = format!("{:#}", manifest(extra.clone()).unwrap_err());
            assert!(error.contains("64 lowercase hex"), "{extra}: {error}");
        }
    }

    #[test]
    fn sources_must_be_a_non_empty_list() {
        use serde_json::json;
        let error = format!("{:#}", manifest(json!({"sources": []})).unwrap_err());
        assert!(error.contains("sources must not be empty"), "{error}");
        assert!(parse(json!({"sources": null})).is_err());
        let mut missing = parse(json!({})).expect("parse without sources");
        missing.sources.clear();
        let error = format!("{:#}", missing.validate("test-game", "top").unwrap_err());
        assert!(error.contains("sources must not be empty"), "{error}");
    }
}
