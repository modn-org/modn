//! Signed catalog key rotations and offline revocations.
//!
//! `rotation.json` and `revocation.json` hold the statement body. The matching
//! `.sig` file holds a minisign signature over a domain-separated statement that
//! binds the catalog name. `revocation.pub` names the offline revocation key.
//! All of these files are inside the catalog digest, so `catalog.sig` covers the
//! exact bytes that a sync reads.

use super::signature::{CatalogKeyId, CatalogPublicKey};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Root file naming the optional offline revocation public key.
pub const REVOCATION_KEY_FILE: &str = "revocation.pub";
/// Root file holding a rotation statement.
pub const ROTATION_FILE: &str = "rotation.json";
/// Root file holding the signature over `rotation.json`.
pub const ROTATION_SIGNATURE_FILE: &str = "rotation.sig";
/// Root file holding a revocation statement.
pub const REVOCATION_FILE: &str = "revocation.json";
/// Root file holding the signature over `revocation.json`.
pub const REVOCATION_SIGNATURE_FILE: &str = "revocation.sig";

const ROTATION_DOMAIN: &[u8] = b"modn-rotation-v1\0";
const REVOCATION_DOMAIN: &[u8] = b"modn-revocation-v1\0";
const MAX_STATEMENT_BYTES: u64 = 32 * 1024;
const MAX_SIGNATURE_BYTES: u64 = 4 * 1024;
const MAX_PUBLIC_KEY_BYTES: u64 = 4 * 1024;

/// Which accepted statement changed the catalog signing key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum KeyChangeKind {
    /// The pinned signing key named a successor and signed the statement.
    Rotated,
    /// The pinned revocation key revoked the signing key and named a successor.
    Revoked,
}

/// One accepted key change, reported by sync output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct CatalogKeyChange {
    /// Whether the pinned signing key rotated or the revocation key revoked it.
    pub kind: KeyChangeKind,
    /// Fingerprint of the key that signed the previous revisions.
    pub from: String,
    /// Fingerprint of the key pinned by this revision.
    pub to: String,
}

impl CatalogKeyChange {
    /// Human-readable line that names both fingerprints.
    #[must_use]
    pub fn line(&self) -> String {
        match self.kind {
            KeyChangeKind::Rotated => {
                format!("signing key rotated: {} -> {}", self.from, self.to)
            }
            KeyChangeKind::Revoked => {
                format!("signing key revoked: {} replaced by {}", self.from, self.to)
            }
        }
    }
}

/// Build the exact `rotation.json` bytes for one statement.
pub fn rotation_statement(from: &str, to_public_key: &str) -> Result<Vec<u8>> {
    serde_json::to_vec(&RotationOut {
        schema_version: 1,
        kind: "rotation",
        from,
        to: to_public_key,
    })
    .context("cannot build the rotation statement")
}

/// Build the exact `revocation.json` bytes for one statement.
pub fn revocation_statement(revoked: &str, replacement_public_key: &str) -> Result<Vec<u8>> {
    serde_json::to_vec(&RevocationOut {
        schema_version: 1,
        kind: "revocation",
        revoked,
        replacement: replacement_public_key,
    })
    .context("cannot build the revocation statement")
}

/// Build the exact bytes signed by `rotation.sig` or `revocation.sig`.
#[must_use]
pub fn key_statement_bytes(kind: KeyChangeKind, catalog_name: &str, statement: &[u8]) -> Vec<u8> {
    let domain = match kind {
        KeyChangeKind::Rotated => ROTATION_DOMAIN,
        KeyChangeKind::Revoked => REVOCATION_DOMAIN,
    };
    let mut bytes = Vec::with_capacity(domain.len() + catalog_name.len() + statement.len() + 12);
    bytes.extend_from_slice(domain);
    bytes.extend_from_slice(b"catalog: ");
    bytes.extend_from_slice(catalog_name.as_bytes());
    bytes.push(b'\n');
    bytes.extend_from_slice(statement);
    bytes
}

#[derive(Serialize)]
struct RotationOut<'a> {
    schema_version: u32,
    kind: &'static str,
    from: &'a str,
    to: &'a str,
}

#[derive(Serialize)]
struct RevocationOut<'a> {
    schema_version: u32,
    kind: &'static str,
    revoked: &'a str,
    replacement: &'a str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RotationBody {
    schema_version: u32,
    kind: String,
    from: String,
    to: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevocationBody {
    schema_version: u32,
    kind: String,
    revoked: String,
    replacement: String,
}

/// Keys pinned by the previous registration.
pub(super) struct PinnedKeys<'a> {
    /// The signing key that verified the previous revision.
    pub(super) signer: &'a CatalogPublicKey,
    /// The offline revocation key pinned at first sync, when the catalog named one.
    pub(super) revocation: Option<&'a CatalogPublicKey>,
    /// BLAKE3 of the rotation statement already applied to this pin.
    pub(super) applied_rotation: Option<&'a str>,
    /// BLAKE3 of the revocation statement already applied to this pin.
    pub(super) applied_revocation: Option<&'a str>,
}

/// An accepted statement and the key that this revision must be signed by.
#[derive(Debug)]
pub(super) struct VerifiedKeyChange {
    pub(super) kind: KeyChangeKind,
    pub(super) from: String,
    pub(super) to: String,
    pub(super) body_digest: String,
    pub(super) new_key: CatalogPublicKey,
}

impl VerifiedKeyChange {
    pub(super) fn report(&self) -> CatalogKeyChange {
        CatalogKeyChange {
            kind: self.kind,
            from: self.from.clone(),
            to: self.to.clone(),
        }
    }
}

/// Read `revocation.pub`, if the catalog publishes one.
pub(super) fn read_revocation_public_key(
    root: &Path,
) -> Result<Option<(CatalogPublicKey, String)>> {
    let Some(bytes) = read_bounded(&root.join(REVOCATION_KEY_FILE), MAX_PUBLIC_KEY_BYTES)? else {
        return Ok(None);
    };
    let text = String::from_utf8(bytes)
        .map_err(|_| anyhow!("catalog revocation key is not UTF-8: {}", root.display()))?;
    let key = CatalogPublicKey::parse(&text).context("malformed catalog revocation key")?;
    Ok(Some((key, text)))
}

/// Refuse a revision that changes or removes the pinned revocation key, or adds one later.
pub(super) fn check_revocation_key(
    name: &str,
    pinned: Option<&CatalogPublicKey>,
    present: Option<&CatalogPublicKey>,
) -> Result<()> {
    match (pinned, present) {
        (Some(pinned), Some(present)) if pinned.key_id() == present.key_id() => Ok(()),
        (Some(pinned), Some(present)) => bail!(
            "Revocation key changed from {} to {} for bucket {name}. The revocation key cannot be changed in place; remove the bucket and sync it again to pin the new key.",
            pinned.key_id(),
            present.key_id()
        ),
        (Some(pinned), None) => bail!(
            "bucket {name} pinned revocation key {} at first sync, but this revision removes revocation.pub. The revocation key cannot be changed in place; restore it or remove the bucket and sync it again.",
            pinned.key_id()
        ),
        (None, Some(present)) => bail!(
            "bucket {name} has no pinned revocation key, but this revision adds revocation.pub for key {}. A revocation key must be present at the first sync; remove the bucket and sync it again to pin it.",
            present.key_id()
        ),
        (None, None) => Ok(()),
    }
}

/// True when the revision carries a rotation or revocation body or signature.
pub(super) fn has_key_statements(root: &Path) -> Result<bool> {
    for file in [
        ROTATION_FILE,
        ROTATION_SIGNATURE_FILE,
        REVOCATION_FILE,
        REVOCATION_SIGNATURE_FILE,
    ] {
        match fs::symlink_metadata(root.join(file)) {
            Ok(_) => return Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(false)
}

/// Verify one rotation or revocation statement and return the key change it names.
pub(super) fn apply_key_statements(
    root: &Path,
    catalog_name: &str,
    pinned: &PinnedKeys<'_>,
    revision_public: Option<&CatalogPublicKey>,
) -> Result<Option<VerifiedKeyChange>> {
    let rotation = read_statement(root, ROTATION_FILE, ROTATION_SIGNATURE_FILE)?;
    let revocation = read_statement(root, REVOCATION_FILE, REVOCATION_SIGNATURE_FILE)?;
    let rotation_applied = rotation
        .as_ref()
        .is_some_and(|(body, _)| is_applied(body, pinned.applied_rotation));
    let revocation_applied = revocation
        .as_ref()
        .is_some_and(|(body, _)| is_applied(body, pinned.applied_revocation));
    match (rotation, revocation) {
        (Some(_), Some(_)) if rotation_applied && revocation_applied => Ok(None),
        (Some(_), Some(_)) => bail!(
            "catalog {catalog_name} contains both rotation.json and revocation.json; a revision may carry only one key change"
        ),
        (Some(_), None) if rotation_applied => Ok(None),
        (None, Some(_)) if revocation_applied => Ok(None),
        (None, None) => Ok(None),
        (Some((body, signature)), None) => {
            Ok(Some(verify_rotation(catalog_name, pinned, revision_public, &body, &signature)?))
        }
        (None, Some((body, signature))) => Ok(Some(verify_revocation(
            catalog_name,
            pinned,
            revision_public,
            &body,
            &signature,
        )?)),
    }
}

fn is_applied(body: &[u8], applied: Option<&str>) -> bool {
    applied.is_some_and(|digest| digest == body_digest(body))
}

fn body_digest(body: &[u8]) -> String {
    blake3::hash(body).to_hex().to_string()
}

fn verify_rotation(
    catalog_name: &str,
    pinned: &PinnedKeys<'_>,
    revision_public: Option<&CatalogPublicKey>,
    body: &[u8],
    signature: &str,
) -> Result<VerifiedKeyChange> {
    let statement: RotationBody = serde_json::from_slice(body)
        .with_context(|| format!("invalid rotation statement for catalog {catalog_name}"))?;
    check_body(
        catalog_name,
        "rotation",
        statement.schema_version,
        &statement.kind,
    )?;
    let pinned_id = pinned.signer.key_id().to_string();
    if !statement.from.eq_ignore_ascii_case(&pinned_id) {
        bail!(
            "rotation statement for catalog {catalog_name} rotates key {}, but the pinned signing key is {pinned_id}",
            statement.from
        );
    }
    let new_key = CatalogPublicKey::parse(&statement.to).with_context(|| {
        format!("rotation statement for catalog {catalog_name} names an invalid key")
    })?;
    let bytes = key_statement_bytes(KeyChangeKind::Rotated, catalog_name, body);
    pinned
        .signer
        .verify_statement(&bytes, signature, "rotation")
        .with_context(|| {
            format!(
                "rotation statement for catalog {catalog_name} is not signed by the pinned signing key {pinned_id}"
            )
        })?;
    require_revision_key(catalog_name, "rotation", new_key.key_id(), revision_public)?;
    Ok(VerifiedKeyChange {
        kind: KeyChangeKind::Rotated,
        from: pinned_id,
        to: new_key.key_id().to_string(),
        body_digest: body_digest(body),
        new_key,
    })
}

fn verify_revocation(
    catalog_name: &str,
    pinned: &PinnedKeys<'_>,
    revision_public: Option<&CatalogPublicKey>,
    body: &[u8],
    signature: &str,
) -> Result<VerifiedKeyChange> {
    let statement: RevocationBody = serde_json::from_slice(body)
        .with_context(|| format!("invalid revocation statement for catalog {catalog_name}"))?;
    check_body(
        catalog_name,
        "revocation",
        statement.schema_version,
        &statement.kind,
    )?;
    let Some(revocation_key) = pinned.revocation else {
        bail!(
            "catalog {catalog_name} has a revocation statement, but no revocation key was pinned at first sync; Modn refuses it"
        );
    };
    let pinned_id = pinned.signer.key_id().to_string();
    if !statement.revoked.eq_ignore_ascii_case(&pinned_id) {
        bail!(
            "revocation statement for catalog {catalog_name} revokes key {}, but the pinned signing key is {pinned_id}",
            statement.revoked
        );
    }
    let new_key = CatalogPublicKey::parse(&statement.replacement).with_context(|| {
        format!("revocation statement for catalog {catalog_name} names an invalid replacement key")
    })?;
    let bytes = key_statement_bytes(KeyChangeKind::Revoked, catalog_name, body);
    revocation_key
        .verify_statement(&bytes, signature, "revocation")
        .with_context(|| {
            format!(
                "revocation statement for catalog {catalog_name} is not signed by the pinned revocation key {}",
                revocation_key.key_id()
            )
        })?;
    require_revision_key(
        catalog_name,
        "revocation",
        new_key.key_id(),
        revision_public,
    )?;
    Ok(VerifiedKeyChange {
        kind: KeyChangeKind::Revoked,
        from: pinned_id,
        to: new_key.key_id().to_string(),
        body_digest: body_digest(body),
        new_key,
    })
}

fn check_body(catalog_name: &str, expected: &str, version: u32, kind: &str) -> Result<()> {
    if version != 1 {
        bail!(
            "{expected} statement for catalog {catalog_name} uses schema_version {version}; Modn supports 1"
        );
    }
    if kind != expected {
        bail!(
            "{expected} statement for catalog {catalog_name} has kind \"{kind}\"; expected \"{expected}\""
        );
    }
    Ok(())
}

fn require_revision_key(
    catalog_name: &str,
    role: &str,
    expected: CatalogKeyId,
    revision_public: Option<&CatalogPublicKey>,
) -> Result<()> {
    let Some(revision_public) = revision_public else {
        bail!(
            "{role} statement for catalog {catalog_name} names key {expected}, but this revision has no catalog.pub; a key change must publish the new public key"
        );
    };
    if revision_public.key_id() != expected {
        bail!(
            "{role} statement for catalog {catalog_name} names key {expected}, but this revision publishes key {}",
            revision_public.key_id()
        );
    }
    Ok(())
}

fn read_statement(
    root: &Path,
    body_file: &str,
    signature_file: &str,
) -> Result<Option<(Vec<u8>, String)>> {
    let body = read_bounded(&root.join(body_file), MAX_STATEMENT_BYTES)?;
    let signature = read_bounded(&root.join(signature_file), MAX_SIGNATURE_BYTES)?;
    match (body, signature) {
        (None, None) => Ok(None),
        (Some(_), None) => bail!("{body_file} is missing its signature {signature_file}"),
        (None, Some(_)) => bail!("{signature_file} has no matching {body_file}"),
        (Some(body), Some(signature)) => {
            let signature = String::from_utf8(signature)
                .map_err(|_| anyhow!("{signature_file} is not UTF-8"))?;
            Ok(Some((body, signature)))
        }
    }
}

fn read_bounded(path: &Path, limit: u64) -> Result<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.file_type().is_file() {
        bail!(
            "catalog key statement must be a regular file: {}",
            path.display()
        );
    }
    if metadata.len() > limit {
        bail!(
            "catalog key statement exceeds size limit: {}",
            path.display()
        );
    }
    Ok(Some(fs::read(path).with_context(|| {
        format!("cannot read catalog key statement: {}", path.display())
    })?))
}

#[cfg(test)]
mod tests;
