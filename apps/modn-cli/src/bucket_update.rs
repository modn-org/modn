use crate::main_release;
use anyhow::{bail, Result};
use modn_core::buckets::{BucketStore, CatalogKeyId};
use std::fs::File;
use std::io::Read;

pub(super) struct MainRefresh {
    pub(super) digest: String,
    pub(super) changed: bool,
    pub(super) key_id: CatalogKeyId,
    pub(super) release: Option<String>,
    pub(super) newer: Option<main_release::UnsupportedRelease>,
}

pub(super) fn source_reader(source: &str) -> Result<Box<dyn Read>> {
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

fn signature_reader(source: &str, signature: &str) -> Result<Box<dyn Read>> {
    match source_reader(signature) {
        Ok(reader) => Ok(reader),
        Err(error) if is_missing(&error) => bail!(
            "The main bucket release at {source} is unsigned; Modn refuses unsigned releases."
        ),
        Err(error) => Err(error),
    }
}

pub(super) fn refresh(roots: &modn_core::manager_roots::ManagerRoots) -> Result<MainRefresh> {
    let (source, signature, selection) = match std::env::var("MODN_MAIN_BUCKET_SOURCE") {
        Ok(source) => {
            let signature = format!("{source}.minisig");
            (source, signature, None)
        }
        Err(_) => {
            let selection = main_release::select()?;
            (
                selection.zip.clone(),
                selection.signature.clone(),
                Some(selection),
            )
        }
    };
    let store = BucketStore::new(roots);
    let updated = store.update_main(
        crate::main_key::MAIN_BUCKET_PUBLIC_KEY,
        || source_reader(&source),
        || signature_reader(&source, &signature),
    )?;
    let (digest, changed, key_id) = (updated.digest, updated.changed, updated.key_id);
    let (release, newer) = match selection {
        Some(selection) => (Some(selection.tag), selection.newer),
        None => (None, None),
    };
    Ok(MainRefresh {
        digest,
        changed,
        key_id,
        release,
        newer,
    })
}

pub(super) fn run(
    name: Option<&str>,
    roots: &modn_core::manager_roots::ManagerRoots,
    json: bool,
) -> Result<()> {
    if name.is_some_and(|name| name != "main") {
        bail!("only main can be updated; registered local buckets are local directories");
    }
    let refreshed = refresh(roots)?;
    if json {
        println!(
            "{}",
            serde_json::json!({
                "bucket":"main",
                "release":refreshed.release,
                "digest":refreshed.digest,
                "changed":refreshed.changed,
                "signed_by":refreshed.key_id.to_string(),
                "newer_release_needs_modn":refreshed.newer.as_ref().map(|newer| &newer.min_modn_version),
            })
        );
    } else {
        println!("Verified main bucket signed by {}.", refreshed.key_id);
        println!(
            "Installed main {}; changed: {}",
            refreshed.digest, refreshed.changed
        );
        if let (Some(release), Some(newer)) = (&refreshed.release, &refreshed.newer) {
            eprintln!("{}", newer.notice(release));
        }
    }
    Ok(())
}
