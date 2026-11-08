use anyhow::{bail, Context, Result};
use std::io::Write;
use std::path::PathBuf;

fn main() -> Result<()> {
    let mut arguments = std::env::args_os().skip(1);
    let root = arguments
        .next()
        .map(PathBuf::from)
        .context("usage: catalog_digest <directory>")?;
    if arguments.next().is_some() {
        bail!("usage: catalog_digest <directory>");
    }
    let statement = modn_core::buckets::catalog_statement(&root)?;
    std::io::stdout().write_all(statement.as_bytes())?;
    std::io::stdout().flush()?;
    Ok(())
}
