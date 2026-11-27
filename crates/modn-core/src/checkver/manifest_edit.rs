//! Order-preserving edits of one package manifest document.
//!
//! `modn catalog checkver --update` rewrites one package manifest. A plain
//! `serde_json::Value` drops the key order of the input, so this module parses
//! the document into an ordered tree and renders it with the indentation and
//! line ending of the original file. Only edited values change in the output.

use crate::buckets::ArchiveFormat;
use anyhow::{bail, Context, Result};
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use std::fmt;
use std::fmt::Write as _;

/// The values one checkver update writes into a package manifest.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ManifestUpdate<'a> {
    /// The new package version.
    pub version: &'a str,
    /// Position of the source to rewrite in `sources`, zero-based.
    pub source_index: usize,
    /// The new selector of that source.
    pub source: SourceUpdate<'a>,
    /// The archive format of the new release, for an archive source.
    pub archive_format: Option<ArchiveFormat>,
    /// The new BLAKE3 archive pin, for an archive source.
    pub blake3: Option<&'a str>,
    /// Drop `upstream_sha256`, which describes the previous artifact.
    pub remove_upstream_sha256: bool,
}

/// The source fields a checkver update rewrites.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SourceUpdate<'a> {
    /// A GitHub release selector.
    GithubRelease {
        /// The exact release tag.
        tag: &'a str,
        /// The exact asset name.
        asset: &'a str,
    },
    /// A Git source commit.
    Git {
        /// The full lowercase 40-character commit.
        commit: &'a str,
    },
}

impl<'a> SourceUpdate<'a> {
    /// A GitHub release selector update.
    #[must_use]
    pub fn github_release(tag: &'a str, asset: &'a str) -> Self {
        Self::GithubRelease { tag, asset }
    }

    /// A Git commit update.
    #[must_use]
    pub fn git(commit: &'a str) -> Self {
        Self::Git { commit }
    }
}

impl<'a> ManifestUpdate<'a> {
    /// An update that rewrites `version` and one source.
    #[must_use]
    pub fn new(version: &'a str, source_index: usize, source: SourceUpdate<'a>) -> Self {
        Self {
            version,
            source_index,
            source,
            archive_format: None,
            blake3: None,
            remove_upstream_sha256: false,
        }
    }

    /// Also rewrite `archive_format` and `blake3`.
    #[must_use]
    pub fn with_archive(mut self, format: ArchiveFormat, blake3: &'a str) -> Self {
        self.archive_format = Some(format);
        self.blake3 = Some(blake3);
        self
    }

    /// Also drop the stale `upstream_sha256` provenance value.
    #[must_use]
    pub fn dropping_upstream_sha256(mut self) -> Self {
        self.remove_upstream_sha256 = true;
        self
    }
}

/// The archive format that a release asset name names.
///
/// # Errors
///
/// Returns an error when the name ends with neither `.zip` nor `.7z`.
pub fn archive_format_for_asset(asset: &str) -> Result<ArchiveFormat> {
    let lower = asset.to_ascii_lowercase();
    if lower.ends_with(".zip") {
        Ok(ArchiveFormat::Zip)
    } else if lower.ends_with(".7z") {
        Ok(ArchiveFormat::SevenZ)
    } else {
        bail!("release asset {asset} is neither a .zip nor a .7z archive")
    }
}

/// Rewrite one manifest document, keeping its key order and layout.
///
/// # Errors
///
/// Returns an error when the document is not a JSON object, lacks a field the
/// update edits, or names a source position that does not exist.
pub fn rewrite_manifest(text: &str, update: &ManifestUpdate<'_>) -> Result<String> {
    let mut root: Node = serde_json::from_str(text).context("invalid package manifest JSON")?;
    let object = object_mut(&mut root, "manifest")?;
    set_string(object, "version", update.version.to_owned())?;
    if let Some(format) = &update.archive_format {
        let value = match format {
            ArchiveFormat::Zip => "zip",
            ArchiveFormat::SevenZ => "7z",
        };
        set_string(object, "archive_format", value.to_owned())?;
    }
    if let Some(blake3) = update.blake3 {
        set_string(object, "blake3", blake3.to_owned())?;
    }
    if update.remove_upstream_sha256 {
        object.retain(|(name, _)| name != "upstream_sha256");
    }
    let sources = match object.iter_mut().find(|(name, _)| name == "sources") {
        Some((_, Node::Array(sources))) => sources,
        Some(_) => bail!("manifest sources must be a JSON array"),
        None => bail!("manifest has no sources field"),
    };
    let source = sources
        .get_mut(update.source_index)
        .with_context(|| format!("manifest has no source at index {}", update.source_index))?;
    let source = object_mut(source, "source")?;
    match &update.source {
        SourceUpdate::GithubRelease { tag, asset } => {
            set_string(source, "tag", (*tag).to_owned())?;
            set_string(source, "asset", (*asset).to_owned())?;
        }
        SourceUpdate::Git { commit } => {
            set_string(source, "commit", (*commit).to_owned())?;
        }
    }
    let style = Style::detect(text);
    Ok(style.render(&root))
}

fn object_mut<'a>(node: &'a mut Node, what: &str) -> Result<&'a mut Vec<(String, Node)>> {
    match node {
        Node::Object(fields) => Ok(fields),
        _ => bail!("manifest {what} must be a JSON object"),
    }
}

fn set_string(fields: &mut Vec<(String, Node)>, key: &str, value: String) -> Result<()> {
    match fields.iter_mut().find(|(name, _)| name == key) {
        Some((_, field)) => {
            *field = Node::String(value);
            Ok(())
        }
        None => bail!("manifest has no {key} field"),
    }
}

struct Style {
    indent: usize,
    line_ending: &'static str,
    trailing_newline: bool,
}

impl Style {
    fn detect(text: &str) -> Self {
        let line_ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let indent = text
            .split('\n')
            .nth(1)
            .map(|line| line.bytes().take_while(|byte| *byte == b' ').count())
            .filter(|spaces| *spaces >= 4)
            .map_or(2, |_| 4);
        Self {
            indent,
            line_ending,
            trailing_newline: text.ends_with('\n'),
        }
    }

    fn render(&self, root: &Node) -> String {
        let mut output = String::new();
        write_node(root, &mut output, self.indent, 0);
        if self.line_ending == "\r\n" {
            output = output.replace('\n', "\r\n");
        }
        if self.trailing_newline {
            output.push_str(self.line_ending);
        }
        output
    }
}

fn write_node(node: &Node, output: &mut String, indent: usize, depth: usize) {
    match node {
        Node::Null => output.push_str("null"),
        Node::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Node::Number(number) => {
            let _ = write!(output, "{number}");
        }
        Node::String(value) => {
            output.push_str(&serde_json::to_string(value).expect("a JSON string serializes"));
        }
        Node::Array(items) if items.is_empty() => output.push_str("[]"),
        Node::Array(items) => {
            output.push_str("[\n");
            for (index, item) in items.iter().enumerate() {
                indent_to(output, indent, depth + 1);
                write_node(item, output, indent, depth + 1);
                if index + 1 < items.len() {
                    output.push(',');
                }
                output.push('\n');
            }
            indent_to(output, indent, depth);
            output.push(']');
        }
        Node::Object(fields) if fields.is_empty() => output.push_str("{}"),
        Node::Object(fields) => {
            output.push_str("{\n");
            for (index, (name, value)) in fields.iter().enumerate() {
                indent_to(output, indent, depth + 1);
                output.push_str(&serde_json::to_string(name).expect("a JSON string serializes"));
                output.push_str(": ");
                write_node(value, output, indent, depth + 1);
                if index + 1 < fields.len() {
                    output.push(',');
                }
                output.push('\n');
            }
            indent_to(output, indent, depth);
            output.push('}');
        }
    }
}

fn indent_to(output: &mut String, indent: usize, depth: usize) {
    for _ in 0..indent * depth {
        output.push(' ');
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Node {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Self>),
    Object(Vec<(String, Self)>),
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(NodeVisitor)
    }
}

struct NodeVisitor;

impl<'de> Visitor<'de> for NodeVisitor {
    type Value = Node;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Node::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(Node::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(Node::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(Node::Number)
            .ok_or_else(|| E::custom("JSON number is not finite"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(Node::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(Node::String(value))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Node::Null)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Node::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Node::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut items = Vec::new();
        while let Some(item) = sequence.next_element()? {
            items.push(item);
        }
        Ok(Node::Array(items))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut fields = Vec::new();
        while let Some((name, value)) = map.next_entry()? {
            fields.push((name, value));
        }
        Ok(Node::Object(fields))
    }
}

#[cfg(test)]
mod tests {
    use super::{archive_format_for_asset, rewrite_manifest, ManifestUpdate, SourceUpdate};
    use crate::buckets::ArchiveFormat;

    #[test]
    fn checkver_archive_format_follows_the_asset_extension() {
        assert!(matches!(
            archive_format_for_asset("Mod-1.0.zip"),
            Ok(ArchiveFormat::Zip)
        ));
        assert!(matches!(
            archive_format_for_asset("Mod-1.0.ZIP"),
            Ok(ArchiveFormat::Zip)
        ));
        assert!(matches!(
            archive_format_for_asset("Mod-1.0.7z"),
            Ok(ArchiveFormat::SevenZ)
        ));
        for asset in ["Mod-1.0.rar", "Mod-1.0", "Mod-1.0.zip.sig"] {
            let error = archive_format_for_asset(asset).unwrap_err().to_string();
            assert!(error.contains("neither a .zip nor a .7z"), "{error}");
        }
    }

    #[test]
    fn checkver_manifest_edit_keeps_key_order_four_spaces_and_newline() {
        let original = "{\n    \"schema_version\": 2,\n    \"id\": \"top\",\n    \"version\": \"1.0.0\",\n    \"archive_format\": \"zip\",\n    \"blake3\": \"0\",\n    \"upstream_sha256\": \"1\",\n    \"sources\": [\n        {\n            \"kind\": \"github-release\",\n            \"repository\": \"owner/repo\",\n            \"tag\": \"v1.0.0\",\n            \"asset\": \"Top-1.0.0.zip\"\n        },\n        {\n            \"kind\": \"https\",\n            \"url\": \"https://example.invalid/top.zip\"\n        }\n    ]\n}\n";
        let update = ManifestUpdate {
            version: "1.1.0",
            source_index: 0,
            source: SourceUpdate::GithubRelease {
                tag: "v1.1.0",
                asset: "Top-1.1.0.7z",
            },
            archive_format: Some(ArchiveFormat::SevenZ),
            blake3: Some("a"),
            remove_upstream_sha256: true,
        };
        let rewritten = rewrite_manifest(original, &update).expect("rewrite");
        let expected = "{\n    \"schema_version\": 2,\n    \"id\": \"top\",\n    \"version\": \"1.1.0\",\n    \"archive_format\": \"7z\",\n    \"blake3\": \"a\",\n    \"sources\": [\n        {\n            \"kind\": \"github-release\",\n            \"repository\": \"owner/repo\",\n            \"tag\": \"v1.1.0\",\n            \"asset\": \"Top-1.1.0.7z\"\n        },\n        {\n            \"kind\": \"https\",\n            \"url\": \"https://example.invalid/top.zip\"\n        }\n    ]\n}\n";
        assert_eq!(rewritten, expected);
    }

    #[test]
    fn checkver_manifest_edit_defaults_to_two_spaces_without_a_newline() {
        let original = "{\"version\":\"1.0\",\"sources\":[{\"kind\":\"git\",\"commit\":\"a\"}]}";
        let update = ManifestUpdate {
            version: "git.b",
            source_index: 0,
            source: SourceUpdate::Git { commit: "b" },
            archive_format: None,
            blake3: None,
            remove_upstream_sha256: false,
        };
        let rewritten = rewrite_manifest(original, &update).expect("rewrite");
        assert_eq!(
            rewritten,
            "{\n  \"version\": \"git.b\",\n  \"sources\": [\n    {\n      \"kind\": \"git\",\n      \"commit\": \"b\"\n    }\n  ]\n}"
        );
    }
}
