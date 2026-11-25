use super::keys::{
    fingerprint, load_secret_outside, read_public_key_text, require_directory, write_signature,
};
use super::SIGNATURE_COMMENT;
use anyhow::{anyhow, bail, Context, Result};
use minisign::{PublicKey, SecretKey};
use modn_core::buckets::{
    catalog_statement, key_statement_bytes, revocation_statement, rotation_statement,
    CatalogPublicKey, KeyChangeKind,
};
use serde_json::json;
use std::io::Cursor;
use std::path::Path;

const REVOCATION_FILE: &str = "revocation.json";
const REVOCATION_SIGNATURE_FILE: &str = "revocation.sig";
const ROTATION_FILE: &str = "rotation.json";
const ROTATION_SIGNATURE_FILE: &str = "rotation.sig";

pub(super) fn rotate(
    root: &Path,
    key_path: &Path,
    new_public_key_path: &Path,
    password_env: Option<&str>,
    json_output: bool,
) -> Result<()> {
    require_directory(root, "catalog directory")?;
    let secret = load_secret_outside(root, key_path, password_env)?;
    let old_public = PublicKey::from_secret_key(&secret)
        .map_err(|_| anyhow!("secret key does not contain its public key"))?;
    let from = fingerprint(&old_public)?;
    let new_text = read_public_key_text(new_public_key_path)?;
    let new_text = new_text.trim().to_owned();
    let new_key = CatalogPublicKey::parse(&new_text)
        .with_context(|| format!("invalid new public key: {}", new_public_key_path.display()))?;
    if new_key.key_id().to_string() == from {
        bail!("a rotation must name a different signing key; the new public key is {from}");
    }
    let to = new_key.key_id().to_string();
    let (view, _staging) = super::published_view(root, false)?;
    let name = catalog_statement(&view)
        .context("cannot read the catalog")?
        .name()
        .to_owned();
    let body = rotation_statement(&from, &new_text)?;
    let bytes = key_statement_bytes(KeyChangeKind::Rotated, &name, &body);
    let signature = sign_statement(&old_public, &secret, &bytes, "rotation")?;
    write_signature(&root.join(ROTATION_FILE), &body)?;
    write_signature(&root.join(ROTATION_SIGNATURE_FILE), signature.as_bytes())?;
    if json_output {
        println!(
            "{}",
            json!({
                "catalog": name,
                "from": from,
                "to": to,
                "rotation": root.join(ROTATION_FILE),
                "signature": root.join(ROTATION_SIGNATURE_FILE),
            })
        );
    } else {
        println!("Wrote a rotation statement for catalog {name}: {from} -> {to}");
        println!(
            "Next, run `modn catalog sign {} --key <new-secret-key>` to sign the revision with the new key.",
            root.display()
        );
    }
    Ok(())
}

pub(super) fn revoke(
    root: &Path,
    revocation_key_path: &Path,
    revoked: &str,
    replacement_public_key_path: &Path,
    password_env: Option<&str>,
    json_output: bool,
) -> Result<()> {
    require_directory(root, "catalog directory")?;
    let revoked = normalize_key_id(revoked)?;
    let secret = load_secret_outside(root, revocation_key_path, password_env)?;
    let revocation_public = PublicKey::from_secret_key(&secret)
        .map_err(|_| anyhow!("revocation key does not contain its public key"))?;
    let revocation_fingerprint = fingerprint(&revocation_public)?;
    let replacement_text = read_public_key_text(replacement_public_key_path)?;
    let replacement_text = replacement_text.trim().to_owned();
    let replacement = CatalogPublicKey::parse(&replacement_text).with_context(|| {
        format!(
            "invalid replacement public key: {}",
            replacement_public_key_path.display()
        )
    })?;
    if replacement.key_id().to_string() == revoked {
        bail!("a revocation must name a different replacement key; the revoked key is {revoked}");
    }
    let (view, _staging) = super::published_view(root, false)?;
    let name = catalog_statement(&view)
        .context("cannot read the catalog")?
        .name()
        .to_owned();
    let body = revocation_statement(&revoked, &replacement_text)?;
    let bytes = key_statement_bytes(KeyChangeKind::Revoked, &name, &body);
    let signature = sign_statement(&revocation_public, &secret, &bytes, "revocation")?;
    write_signature(&root.join(REVOCATION_FILE), &body)?;
    write_signature(&root.join(REVOCATION_SIGNATURE_FILE), signature.as_bytes())?;
    let replacement_fingerprint = replacement.key_id().to_string();
    if json_output {
        println!(
            "{}",
            json!({
                "catalog": name,
                "revocation_key_id": revocation_fingerprint,
                "revoked": revoked,
                "replacement": replacement_fingerprint,
                "revocation": root.join(REVOCATION_FILE),
                "signature": root.join(REVOCATION_SIGNATURE_FILE),
            })
        );
    } else {
        println!(
            "Wrote a revocation statement for catalog {name}: {revoked} replaced by {replacement_fingerprint}"
        );
        println!(
            "Next, run `modn catalog sign {} --key <replacement-secret-key>` to sign the revision with the replacement key.",
            root.display()
        );
    }
    Ok(())
}

fn sign_statement(
    public: &PublicKey,
    secret: &SecretKey,
    bytes: &[u8],
    role: &str,
) -> Result<String> {
    minisign::sign(
        Some(public),
        secret,
        Cursor::new(bytes),
        Some(SIGNATURE_COMMENT),
        Some(SIGNATURE_COMMENT),
    )
    .map(|signature| signature.into_string())
    .map_err(|error| anyhow!("cannot sign the {role} statement: {error}"))
}

fn normalize_key_id(text: &str) -> Result<String> {
    let text = text.trim();
    if text.len() != 16 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("--revoked must be a 16-digit hexadecimal key fingerprint: {text}");
    }
    Ok(text.to_ascii_uppercase())
}
