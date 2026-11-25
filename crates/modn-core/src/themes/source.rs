#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;

use serde::Serialize;

/// The three kinds of theme source in `docs/themes.md`.
///
/// The derived ordering is the enum order, not the precedence order. Use
/// [`ThemeSourceKind::precedence`] to compare sources.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeSourceKind {
    /// The official `main` bucket.
    Official,
    /// Any trusted bucket other than `main`.
    Bucket,
    /// The local theme registry.
    Local,
}

impl ThemeSourceKind {
    /// Return the documented precedence rank: official 0, local 1, bucket 2.
    ///
    /// A smaller rank wins when two sources hold the same `(game, id)` pair.
    #[must_use]
    pub(crate) fn precedence(self) -> u8 {
        match self {
            Self::Official => 0,
            Self::Local => 1,
            Self::Bucket => 2,
        }
    }
}

/// One theme source in the CLI shape.
///
/// Serializes to `{"kind":"official"|"bucket"|"local","bucket":"main"|"<id>"|null,
/// "label":"main bucket"|"<id>"|"local registry"}`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThemeSource {
    /// Which kind of source holds the theme.
    pub kind: ThemeSourceKind,
    /// Bucket id, or `None` for the local registry.
    pub bucket: Option<String>,
    /// Human-readable source label.
    pub label: String,
}

impl ThemeSource {
    /// The official `main` bucket source.
    #[must_use]
    pub fn official() -> Self {
        Self {
            kind: ThemeSourceKind::Official,
            bucket: Some("main".to_owned()),
            label: "main bucket".to_owned(),
        }
    }

    /// One trusted bucket source, labelled by its id.
    #[must_use]
    pub fn bucket(id: impl Into<String>) -> Self {
        let id = id.into();
        Self {
            kind: ThemeSourceKind::Bucket,
            bucket: Some(id.clone()),
            label: id,
        }
    }

    /// The local theme registry source.
    #[must_use]
    pub fn local() -> Self {
        Self {
            kind: ThemeSourceKind::Local,
            bucket: None,
            label: "local registry".to_owned(),
        }
    }

    /// Which kind of source holds the theme.
    #[must_use]
    pub fn kind(&self) -> ThemeSourceKind {
        self.kind
    }

    /// Bucket id, or `None` for the local registry.
    #[must_use]
    pub fn bucket_id(&self) -> Option<&str> {
        self.bucket.as_deref()
    }

    /// Human-readable source label.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }
}
