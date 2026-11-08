//! Sign a catalog directory with an ephemeral key for smoke tests.
//!
//! The key pair lives only in memory. Print the key identifier so a caller can
//! record which key signed the catalog. Never write the secret key to disk.
use anyhow::{bail, Context, Result};
use std::io::Cursor;
use std::path::PathBuf;

const USAGE: &str = "usage: sign_catalog <directory> <signature-out> <public-key-out>";

fn main() -> Result<()> {
    let mut arguments = std::env::args_os().skip(1);
    let root = arguments.next().map(PathBuf::from).with_context(|| USAGE)?;
    let signature_out = arguments.next().map(PathBuf::from).with_context(|| USAGE)?;
    let public_key_out = arguments.next().map(PathBuf::from).with_context(|| USAGE)?;
    if arguments.next().is_some() {
        bail!("{USAGE}");
    }
    let statement = modn_core::buckets::catalog_statement(&root)?;
    let pair = minisign::KeyPair::generate_unencrypted_keypair()
        .context("cannot generate the ephemeral test key")?;
    let signature = minisign::sign(
        Some(&pair.pk),
        &pair.sk,
        Cursor::new(statement.as_bytes()),
        Some("modn smoke test bucket"),
        Some("modn smoke test bucket signature"),
    )
    .context("cannot sign the catalog statement")?;
    let public_key = pair
        .pk
        .to_box()
        .context("cannot encode the ephemeral public key")?;
    std::fs::write(&signature_out, signature.into_string())
        .with_context(|| format!("cannot write {}", signature_out.display()))?;
    std::fs::write(&public_key_out, public_key.into_string())
        .with_context(|| format!("cannot write {}", public_key_out.display()))?;
    let key_id = pair
        .pk
        .keynum()
        .get(..8)
        .and_then(|bytes| <[u8; 8]>::try_from(bytes).ok())
        .map(u64::from_le_bytes)
        .context("ephemeral public key has no key identifier")?;
    println!("{key_id:016X}");
    Ok(())
}
