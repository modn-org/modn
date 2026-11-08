use anyhow::{bail, Result};
use modn_core::buckets::BucketStore;
use std::fs::File;
use std::io::Read;

const DEFAULT_SOURCE: &str =
    "https://github.com/modn-org/main/releases/latest/download/main-bucket.zip";

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

fn is_missing(error: &anyhow::Error) -> bool {
    if let Some(error) = error.downcast_ref::<std::io::Error>() {
        return error.kind() == std::io::ErrorKind::NotFound;
    }
    matches!(
        error.downcast_ref::<ureq::Error>(),
        Some(ureq::Error::StatusCode(404))
    )
}

fn signature_reader(source: &str) -> Result<Box<dyn Read>> {
    match source_reader(&format!("{source}.minisig")) {
        Ok(reader) => Ok(reader),
        Err(error) if is_missing(&error) => bail!(
            "The main bucket release at {source} is unsigned; Modn refuses unsigned releases."
        ),
        Err(error) => Err(error),
    }
}

pub(super) fn run(name: Option<&str>, json: bool) -> Result<()> {
    if name.is_some_and(|name| name != "main") {
        bail!("only main can be updated; registered local buckets are local directories");
    }
    let source = std::env::var("MODN_MAIN_BUCKET_SOURCE").unwrap_or_else(|_| DEFAULT_SOURCE.into());
    let store = BucketStore::new(&modn_core::data_dir::data_directory()?);
    let (digest, changed, key_id) = store.update_main(
        crate::main_key::MAIN_BUCKET_PUBLIC_KEY,
        || source_reader(&source),
        || signature_reader(&source),
    )?;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "bucket":"main",
                "digest":digest,
                "changed":changed,
                "signed_by":key_id.to_string(),
            })
        );
    } else {
        println!("Verified main bucket signed by {key_id}.");
        println!("Installed main {digest}; changed: {changed}");
    }
    Ok(())
}
