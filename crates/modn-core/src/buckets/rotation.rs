//! Signed catalog key rotations and offline revocations.
//!
//! `rotation.json` and `revocation.json` hold the statement body. The matching
//! `.sig` file holds a minisign signature over a domain-separated statement that
//! binds the catalog name. `revocation.pub` names the offline revocation key.
//! All of these files are inside the catalog digest, so `catalog.sig` covers the
//! exact bytes that a sync reads. A sync applies a statement only when it names
//! the pinned key and the matching key signed it.

use super::signature::CatalogPublicKey;
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

/// Build the exact `rotation.json` bytes for one statement without a sequence.
pub fn rotation_statement(from: &str, to_public_key: &str) -> Result<Vec<u8>> {
    rotation_statement_with_sequence(from, to_public_key, None)
}

/// Build the exact `rotation.json` bytes for one statement.
///
/// A statement that carries a sequence must name the catalog revision's
/// sequence number, so a replayed statement cannot hide in a newer revision.
pub fn rotation_statement_with_sequence(
    from: &str,
    to_public_key: &str,
    sequence: impl Into<Option<u64>>,
) -> Result<Vec<u8>> {
    serde_json::to_vec(&RotationOut {
        schema_version: 1,
        kind: "rotation",
        from,
        to: to_public_key,
        sequence: sequence.into(),
    })
    .context("cannot build the rotation statement")
}

/// Build the exact `revocation.json` bytes for one statement without a sequence.
pub fn revocation_statement(revoked: &str, replacement_public_key: &str) -> Result<Vec<u8>> {
    revocation_statement_with_sequence(revoked, replacement_public_key, None)
}

/// Build the exact `revocation.json` bytes for one statement.
///
/// A statement that carries a sequence must name the catalog revision's
/// sequence number, so a replayed statement cannot hide in a newer revision.
pub fn revocation_statement_with_sequence(
    revoked: &str,
    replacement_public_key: &str,
    sequence: impl Into<Option<u64>>,
) -> Result<Vec<u8>> {
    serde_json::to_vec(&RevocationOut {
        schema_version: 1,
        kind: "revocation",
        revoked,
        replacement: replacement_public_key,
        sequence: sequence.into(),
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
    #[serde(skip_serializing_if = "Option::is_none")]
    sequence: Option<u64>,
}

#[derive(Serialize)]
struct RevocationOut<'a> {
    schema_version: u32,
    kind: &'static str,
    revoked: &'a str,
    replacement: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    sequence: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RotationBody {
    schema_version: u32,
    kind: String,
    from: String,
    to: String,
    #[serde(default)]
    sequence: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevocationBody {
    schema_version: u32,
    kind: String,
    revoked: String,
    replacement: String,
    #[serde(default)]
    sequence: Option<u64>,
}

/// Keys pinned by the previous registration.
pub(super) struct PinnedKeys<'a> {
    /// The signing key that verified the previous revision.
    pub(super) signer: &'a CatalogPublicKey,
    /// The offline revocation key pinned at first sync, when the catalog named one.
    pub(super) revocation: Option<&'a CatalogPublicKey>,
    /// BLAKE3 digests of the statements this pin already applied.
    pub(super) applied: Vec<&'a str>,
}

/// An accepted statement and the key that this revision must be signed by.
#[derive(Debug)]
pub(super) struct VerifiedKeyChange {
    pub(super) kind: KeyChangeKind,
    pub(super) from: String,
    pub(super) to: String,
    pub(super) body_digest: String,
    pub(super) new_key: CatalogPublicKey,
    /// The public key text carried by the signed statement.
    pub(super) new_key_text: String,
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

/// What this revision's published statements mean for the pinned key.
#[derive(Debug)]
pub(super) struct KeyStatementOutcome {
    /// The one statement that applies to the pinned key, when present.
    pub(super) change: Option<VerifiedKeyChange>,
    /// Published statement kinds that add no trust to this pin.
    pub(super) ignored: Vec<String>,
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
        (Some(pinned), Some(present)) if pinned == present => Ok(()),
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

/// The key identifier text that a statement names in `from` or `revoked`.
fn key_id_text(catalog_name: &str, kind: &str, field: &str, value: &str) -> Result<String> {
    let value = value.trim();
    if value.len() != 16 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!(
            "{kind} statement for catalog {catalog_name} has an invalid key ID in {field}: {value}"
        );
    }
    Ok(value.to_ascii_uppercase())
}

mod statements;
pub(in crate::buckets) use self::statements::{
    apply_key_statements, validate_historical_statements,
};

#[cfg(test)]
mod tests;
