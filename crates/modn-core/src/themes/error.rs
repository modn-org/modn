#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;

use super::problem::ThemeProblem;
use std::error::Error;
use std::fmt;
use std::path::PathBuf;

/// Result type for theme registry and selection operations.
pub type Result<T> = std::result::Result<T, ThemeError>;

/// A theme registry or selection operation failed.
///
/// A failed operation leaves the stored themes and selections unchanged.
#[non_exhaustive]
#[derive(Debug)]
pub enum ThemeError {
    /// The theme failed inspection, so nothing was copied or stored.
    Invalid(Vec<ThemeProblem>),
    /// A filesystem operation failed.
    Io {
        /// What the operation tried to do.
        context: String,
        /// The underlying failure.
        source: std::io::Error,
    },
    /// A registry or selection file does not parse or fails validation.
    Corrupt {
        /// Path of the offending file.
        path: PathBuf,
        /// Why the file was refused.
        reason: String,
    },
    /// No registration exists for the requested id.
    NotRegistered {
        /// The requested theme id.
        id: String,
    },
    /// The id is registered for more than one game.
    Ambiguous {
        /// The requested theme id.
        id: String,
        /// The games that register the id, sorted.
        games: Vec<String>,
    },
    /// A caller supplied an id that is not valid.
    InvalidId(String),
}

impl ThemeError {
    pub(crate) fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }

    pub(crate) fn corrupt(path: impl Into<PathBuf>, reason: impl Into<String>) -> Self {
        Self::Corrupt {
            path: path.into(),
            reason: reason.into(),
        }
    }
}

impl fmt::Display for ThemeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(problems) => match problems.first() {
                Some(first) => write!(formatter, "the theme failed validation: {first}"),
                None => formatter.write_str("the theme failed validation"),
            },
            Self::Io { context, source } => write!(formatter, "{context}: {source}"),
            Self::Corrupt { path, reason } => {
                write!(formatter, "{} is corrupt: {reason}", path.display())
            }
            Self::NotRegistered { id } => {
                write!(formatter, "no theme with id \"{id}\" is registered")
            }
            Self::Ambiguous { id, games } => write!(
                formatter,
                "theme \"{id}\" is registered for several games ({}); pass the game",
                games.join(", ")
            ),
            Self::InvalidId(id) => write!(formatter, "\"{id}\" is not a valid theme or game id"),
        }
    }
}

impl Error for ThemeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<ThemeProblem> for ThemeError {
    fn from(problem: ThemeProblem) -> Self {
        Self::Invalid(vec![problem])
    }
}
