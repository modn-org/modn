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
    /// Select the theme that one game uses, or select no theme.
    #[command(group(
        clap::ArgGroup::new("choice")
            .args(["theme", "none"])
            .required(true)
    ))]
    Select {
        /// Game id the choice applies to.
        game: String,
        /// Theme id to select.
        theme: Option<String>,
        /// Variant id to apply, or omit for the base stylesheet.
        #[arg(long, value_name = "VARIANT", conflicts_with = "none")]
        variant: Option<String>,
        /// Use the default theme even when the game has an official theme.
        #[arg(long)]
        none: bool,
    },
    /// Clear one game's selection, or every selection.
    Reset,
    /// Resolve the theme that the desktop must apply.
    Resolve,
}
