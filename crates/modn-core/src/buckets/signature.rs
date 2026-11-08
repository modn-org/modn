//! Minisign verification of canonical catalog statements.

use crate::bucket_digest::{catalog_digest, CatalogDigest};
use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use minisign_verify::{PublicKey as MinisignPublicKey, Signature};
use serde::Deserialize;
use std::borrow::Cow;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

const CATALOG_KEY_BYTES: usize = 42;
const CATALOG_SIGNATURE_BYTES: usize = 74;
const MAX_BUCKET_JSON_BYTES: u64 = 32 * 1024;
const MAX_SIGNATURE_BYTES: u64 = 4 * 1024;
const MAIN_KEY_PLACEHOLDER: &str = "UNSET";
const NO_MAIN_KEY: &str = "This Modn build has no main bucket signer key.";

/// Minisign key identifier shown as 16 uppercase hexadecimal digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CatalogKeyId(u64);

impl fmt::Display for CatalogKeyId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:016X}", self.0)
    }
}

/// A minisign public key trusted for one catalog.
#[derive(Clone)]
pub struct CatalogPublicKey {
    verification_key: MinisignPublicKey,
    key_id: CatalogKeyId,
}

impl fmt::Debug for CatalogPublicKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CatalogPublicKey")
            .field("key_id", &self.key_id)
            .finish_non_exhaustive()
    }
}

impl CatalogPublicKey {
    /// Parse a bare minisign public key line or a two-line `minisign.pub` file.
    pub fn parse(text: &str) -> Result<Self> {
        let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
        let first = lines
            .next()
            .ok_or_else(|| anyhow!("malformed catalog public key: empty"))?;
        let encoded = if first.starts_with("untrusted comment:") {
            lines
                .next()
                .ok_or_else(|| anyhow!("malformed catalog public key: missing key line"))?
        } else {
            first
        };
        if lines.next().is_some() {
            bail!("malformed catalog public key: unexpected trailing data");
        }
        if !encoded.starts_with("RW") {
            bail!("malformed catalog public key: invalid key line");
        }
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|_| anyhow!("malformed catalog public key: invalid base64"))?;
        if bytes.len() != CATALOG_KEY_BYTES {
            bail!("malformed catalog public key: invalid length");
        }
        let verification_key = MinisignPublicKey::from_base64(encoded)
            .map_err(|error| anyhow!("malformed catalog public key: {error}"))?;
        Ok(Self {
            verification_key,
            key_id: key_id_from(&bytes)?,
        })
    }

    /// Return the key identifier shown by minisign.
    #[must_use]
    pub fn key_id(&self) -> CatalogKeyId {
        self.key_id
    }
}

/// Exact bytes signed by a catalog signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogStatement {
    name: String,
    digest: CatalogDigest,
    text: String,
}

impl CatalogStatement {
    /// Build the canonical statement for a catalog name and content digest.
    pub fn new(name: String, digest: CatalogDigest) -> Result<Self> {
        if !valid_name(&name) {
            bail!("catalog name must be non-empty ASCII without control characters");
        }
        let text = format!("modn-catalog-signature/v1\ncatalog: {name}\ndigest: {digest}\n");
        Ok(Self { name, digest, text })
    }

    /// Return the catalog name from the root `bucket.json`.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the catalog content digest.
    #[must_use]
    pub fn digest(&self) -> &CatalogDigest {
        &self.digest
    }

    /// Return the statement as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Return the exact bytes to sign or verify.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.text.as_bytes()
    }
}

impl fmt::Display for CatalogStatement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text)
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte == b' ' || byte.is_ascii_graphic())
}

#[derive(Deserialize)]
struct CatalogMetadata {
    name: String,
}

fn catalog_name(root: &Path) -> Result<String> {
    let path = root.join("bucket.json");
    let metadata = fs::symlink_metadata(&path)
        .with_context(|| format!("missing catalog metadata: {}", path.display()))?;
    if !metadata.file_type().is_file() {
        bail!(
            "catalog metadata must be a regular file: {}",
            path.display()
        );
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)?
        .take(MAX_BUCKET_JSON_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BUCKET_JSON_BYTES {
        bail!("catalog metadata exceeds size limit: {}", path.display());
    }
    let metadata: CatalogMetadata = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid catalog metadata: {}", path.display()))?;
    Ok(metadata.name)
}

/// Build the exact signature statement for a catalog directory.
pub fn catalog_statement(root: &Path) -> Result<CatalogStatement> {
    let name = catalog_name(root)?;
    let digest = catalog_digest(root).context("cannot compute catalog digest")?;
    CatalogStatement::new(name, digest)
}

struct ParsedSignature {
    key_id: CatalogKeyId,
    signature: Signature,
}

fn key_id_from(bytes: &[u8]) -> Result<CatalogKeyId> {
    let key_id: [u8; 8] = bytes
        .get(2..10)
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(|| anyhow!("malformed catalog key: missing key identifier"))?;
    Ok(CatalogKeyId(u64::from_le_bytes(key_id)))
}

#[cfg(feature = "test-main-key")]
fn main_bucket_key_text(embedded: &str) -> Cow<'_, str> {
    match std::env::var("MODN_TEST_MAIN_BUCKET_PUBLIC_KEY") {
        Ok(text) if !text.trim().is_empty() => Cow::Owned(text),
        _ => Cow::Borrowed(embedded),
    }
}

#[cfg(not(feature = "test-main-key"))]
fn main_bucket_key_text(embedded: &str) -> Cow<'_, str> {
    Cow::Borrowed(embedded)
}

/// Resolve the main bucket signer key from the text built into Modn.
pub(crate) fn main_bucket_public_key(embedded: &str) -> Result<CatalogPublicKey> {
    let text = main_bucket_key_text(embedded);
    let text = text.trim();
    if text.starts_with(MAIN_KEY_PLACEHOLDER) {
        bail!(NO_MAIN_KEY);
    }
    CatalogPublicKey::parse(text).context("invalid main bucket signer key")
}

fn parse_signature(text: &str) -> Result<ParsedSignature> {
    let mut lines = text.lines();
    let untrusted_comment = lines
        .next()
        .ok_or_else(|| anyhow!("malformed catalog signature: missing untrusted comment"))?;
    let encoded = lines
        .next()
        .ok_or_else(|| anyhow!("malformed catalog signature: missing signature line"))?;
    let trusted_comment = lines
        .next()
        .ok_or_else(|| anyhow!("malformed catalog signature: missing trusted comment"))?;
    let _global_signature = lines
        .next()
        .ok_or_else(|| anyhow!("malformed catalog signature: missing global signature"))?;
    if lines.next().is_some() {
        bail!("malformed catalog signature: unexpected trailing data");
    }
    if !untrusted_comment.starts_with("untrusted comment:")
        || !trusted_comment.starts_with("trusted comment: ")
    {
        bail!("malformed catalog signature: invalid comment lines");
    }
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| anyhow!("malformed catalog signature: invalid base64"))?;
    if bytes.len() != CATALOG_SIGNATURE_BYTES {
        bail!("malformed catalog signature: invalid length");
    }
    if !bytes.starts_with(b"ED") {
        if bytes.starts_with(b"Ed") {
            bail!("legacy catalog signatures are unsupported");
        }
        bail!("malformed catalog signature: unknown algorithm");
    }
    let signature =
        Signature::decode(text).map_err(|error| anyhow!("malformed catalog signature: {error}"))?;
    Ok(ParsedSignature {
        key_id: key_id_from(&bytes)?,
        signature,
    })
}

/// A catalog whose statement was signed by the expected key.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct VerifiedCatalog {
    /// The exact statement that the signature covers.
    pub statement: CatalogStatement,
    /// The identifier of the key that produced the signature.
    pub key_id: CatalogKeyId,
}

/// Verify a minisign signature of the statement for a catalog directory.
pub fn verify_catalog(
    root: &Path,
    key: &CatalogPublicKey,
    signature_text: &str,
) -> Result<VerifiedCatalog> {
    let statement = catalog_statement(root)?;
    let signature = parse_signature(signature_text)?;
    if signature.key_id != key.key_id {
        bail!(
            "catalog signature key {} does not match public key {}",
            signature.key_id,
            key.key_id
        );
    }
    key.verification_key
        .verify(statement.as_bytes(), &signature.signature, false)
        .map_err(|error| anyhow!("catalog signature verification failed: {error}"))?;
    Ok(VerifiedCatalog {
        statement,
        key_id: key.key_id,
    })
}

/// Read `catalog.sig` from a catalog directory and verify it.
pub fn verify_catalog_directory(root: &Path, key: &CatalogPublicKey) -> Result<VerifiedCatalog> {
    let path = root.join("catalog.sig");
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            bail!("catalog signature is missing: {}", path.display())
        }
        Err(error) => return Err(error.into()),
    };
    if !metadata.file_type().is_file() {
        bail!(
            "catalog signature must be a regular file: {}",
            path.display()
        );
    }
    if metadata.len() > MAX_SIGNATURE_BYTES {
        bail!("catalog signature exceeds size limit: {}", path.display());
    }
    let bytes = fs::read(&path)
        .with_context(|| format!("cannot read catalog signature: {}", path.display()))?;
    let text =
        String::from_utf8(bytes).map_err(|_| anyhow!("malformed catalog signature: not UTF-8"))?;
    verify_catalog(root, key, &text)
}

#[cfg(test)]
mod tests;
