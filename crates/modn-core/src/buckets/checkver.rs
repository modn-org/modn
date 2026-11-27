//! Optional upstream selectors for catalog authoring tools.
//!
//! A package manifest without `checkver` is unchanged. Ordinary user commands
//! parse and validate the block but never act on it.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

const MAX_PATTERN_CHARS: usize = 200;
const MAX_BRANCH_CHARS: usize = 255;

/// An upstream release selector for catalog authoring tools.
///
/// The block never affects search, install, update or bucket sync. Catalog
/// authoring tools read it to look for a newer upstream release.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
#[non_exhaustive]
pub enum Checkver {
    /// A GitHub release asset, optionally including prereleases.
    GithubRelease {
        /// Repository in exactly `owner/repo` form.
        repository: String,
        /// Exact asset name, or a pattern where `*` and `?` are wildcards.
        asset_pattern: String,
        /// Whether a GitHub prerelease satisfies the check.
        #[serde(default, skip_serializing_if = "is_false")]
        prerelease: bool,
    },
    /// A branch of a Git repository.
    Git {
        /// Repository in exactly `owner/repo` form.
        repository: String,
        /// Exact branch name.
        branch: String,
    },
    /// A file in a Nexus Mods game domain.
    Nexus {
        /// Nexus game domain, such as `cyberpunk2077`.
        game_domain: String,
        /// Numeric Nexus mod ID.
        mod_id: u64,
        /// Exact file name, or a pattern where `*` and `?` are wildcards.
        file_pattern: String,
    },
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl Checkver {
    /// Validate every selector field in this block.
    ///
    /// `game` and `id` name the package manifest. Every error names the
    /// package and the offending `checkver` field.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty or malformed `repository`, `branch`,
    /// `asset_pattern`, `file_pattern`, `game_domain` or `mod_id`.
    pub fn validate(&self, game: &str, id: &str) -> Result<()> {
        let package = format!("package {game}/{id}");
        match self {
            Self::GithubRelease {
                repository,
                asset_pattern,
                ..
            } => {
                validate_repository(&package, repository)?;
                validate_pattern(&package, "asset_pattern", asset_pattern)?;
            }
            Self::Git { repository, branch } => {
                validate_repository(&package, repository)?;
                if !valid_branch(branch) {
                    bail!(
                        "{package} checkver.branch must be 1 to 255 characters, must not start with `-` or `/`, must not end with `/`, `.` or `.lock`, and must not contain whitespace, control characters, `~`, `^`, `:`, `?`, `*`, `[`, `\\`, `..` or `@{{`"
                    );
                }
            }
            Self::Nexus {
                game_domain,
                mod_id,
                file_pattern,
            } => {
                crate::acquisition::NexusGameDomain::new(game_domain)
                    .with_context(|| format!("{package} checkver.game_domain"))?;
                // Reuse the Nexus source rule, so a selector accepts exactly
                // the mod IDs a pinned manifest source accepts.
                crate::acquisition::NexusFile::new(game_domain, *mod_id, 1, "checkver.zip")
                    .with_context(|| format!("{package} checkver.mod_id"))?;
                validate_pattern(&package, "file_pattern", file_pattern)?;
            }
        }
        Ok(())
    }
}

fn validate_repository(package: &str, repository: &str) -> Result<()> {
    if !valid_repository(repository) {
        bail!(
            "{package} checkver.repository must be exactly owner/repo: two non-empty segments of ASCII letters, digits, `.`, `_` or `-`, and neither segment may be `.` or `..`"
        );
    }
    Ok(())
}

fn valid_repository(repository: &str) -> bool {
    let mut segments = repository.split('/');
    let (Some(owner), Some(name), None) = (segments.next(), segments.next(), segments.next())
    else {
        return false;
    };
    [owner, name].iter().all(|segment| {
        !segment.is_empty()
            && *segment != "."
            && *segment != ".."
            && segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    })
}

fn validate_pattern(package: &str, field: &str, pattern: &str) -> Result<()> {
    if !valid_pattern(pattern) {
        bail!(
            "{package} checkver.{field} must be 1 to 200 characters with no `/`, `\\`, control characters or NUL"
        );
    }
    Ok(())
}

fn valid_pattern(pattern: &str) -> bool {
    let length = pattern.chars().count();
    (1..=MAX_PATTERN_CHARS).contains(&length)
        && !pattern
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\'))
}

fn valid_branch(branch: &str) -> bool {
    let length = branch.chars().count();
    (1..=MAX_BRANCH_CHARS).contains(&length)
        && !branch.starts_with('-')
        && !branch.starts_with('/')
        && !branch.ends_with('/')
        && !branch.ends_with('.')
        && !branch.ends_with(".lock")
        && !branch.contains("..")
        && !branch.contains("@{")
        && !branch.chars().any(|character| {
            character.is_whitespace()
                || character.is_control()
                || matches!(character, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
        })
}

enum JsonKind {
    Text,
    Integer,
    Boolean,
}

/// Name the field of a wrong JSON type before serde reports a bare type error.
fn check_field_types(value: &serde_json::Value) -> Result<(), String> {
    let Some(object) = value.as_object() else {
        return Ok(());
    };
    let Some(kind) = object.get("kind").and_then(serde_json::Value::as_str) else {
        return Ok(());
    };
    let fields: &[(&str, JsonKind)] = match kind {
        "github-release" => &[
            ("repository", JsonKind::Text),
            ("asset_pattern", JsonKind::Text),
            ("prerelease", JsonKind::Boolean),
        ],
        "git" => &[("repository", JsonKind::Text), ("branch", JsonKind::Text)],
        "nexus" => &[
            ("game_domain", JsonKind::Text),
            ("mod_id", JsonKind::Integer),
            ("file_pattern", JsonKind::Text),
        ],
        _ => return Ok(()),
    };
    for (field, expected) in fields {
        let Some(field_value) = object.get(*field) else {
            continue;
        };
        let (valid, expected) = match expected {
            JsonKind::Text => (field_value.is_string(), "a string"),
            JsonKind::Integer => (field_value.as_u64().is_some(), "an integer"),
            JsonKind::Boolean => (field_value.is_boolean(), "a boolean"),
        };
        if !valid {
            return Err(format!("checkver.{field}: expected {expected}"));
        }
    }
    Ok(())
}

/// Deserialize an optional block and qualify shape errors with `checkver`.
pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<Option<Checkver>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    value
        .map(|value| {
            if let Err(error) = check_field_types(&value) {
                return Err(<D::Error as serde::de::Error>::custom(error));
            }
            serde_json::from_value(value).map_err(|error| {
                <D::Error as serde::de::Error>::custom(format!("checkver: {error}"))
            })
        })
        .transpose()
}

#[cfg(test)]
#[path = "checkver_tests.rs"]
mod tests;
