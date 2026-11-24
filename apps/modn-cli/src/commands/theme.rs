//! `modn theme` subcommands: inspect, register, list, select and resolve themes.
//!
//! The global `--game` flag selects one game for `list`, `remove`, `reset` and
//! `resolve`. `select` takes the game and theme positionally, as the desktop
//! spawns it.

use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub(crate) enum ThemeCommand {
    /// Validate one theme directory and report every diagnostic.
    Check {
        /// Theme directory that holds `theme.json` and `theme.css`.
        directory: PathBuf,
    },
    /// Validate and copy one local theme into the theme registry.
    Register {
        /// Theme directory that holds `theme.json` and `theme.css`.
        directory: PathBuf,
    },
    /// List official, bucket and locally registered themes.
    List,
    /// Remove one local theme registration.
    Remove {
        /// Theme id to remove from the local registry.
        theme_id: String,
    },
    /// Select the theme that one game uses.
    Select {
        /// Game id the choice applies to.
        game: String,
        /// Theme id to select.
        theme: String,
        /// Variant id to apply, or omit for the base stylesheet.
        #[arg(long, value_name = "VARIANT")]
        variant: Option<String>,
    },
    /// Clear one game's selection, or every selection.
    Reset,
    /// Resolve the theme that the desktop must apply.
    Resolve,
}
