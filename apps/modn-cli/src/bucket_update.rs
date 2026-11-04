use anyhow::{bail, Result};
use modn_core::buckets::BucketStore;
use std::fs::File;
use std::io::Read;

const DEFAULT_SOURCE: &str =
    "https://github.com/modn-org/modn/releases/latest/download/main-bucket.zip";

fn source_reader(source: &str) -> Result<Box<dyn Read>> {
    if source.starts_with("https://") {
        return Ok(Box::new(crate::download::fetch(source)?));
    }
    if source.contains("://")
        || (source.contains(':') && !std::path::Path::new(source).is_absolute())
    {
        bail!("main bucket source must be an HTTPS URL or a local ZIP file path");
    }
    Ok(Box::new(File::open(source)?))
}

pub(super) fn run(name: Option<&str>, json: bool) -> Result<()> {
    if name.is_some_and(|name| name != "main") {
        bail!("only main can be updated; registered local buckets are local directories");
    }
    let source = std::env::var("MODN_MAIN_BUCKET_SOURCE").unwrap_or_else(|_| DEFAULT_SOURCE.into());
    let store = BucketStore::new(&modn_core::data_dir::data_directory()?);
    let (digest, changed) = store.update_main(
        || source_reader(&source),
        || source_reader(&format!("{source}.sha256")),
    )?;
    if json {
        println!(
            "{}",
            serde_json::json!({"bucket":"main", "digest":digest,"changed":changed})
        );
    } else {
        println!("Installed main {digest}; changed: {changed}");
    }
    Ok(())
}
