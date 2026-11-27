//! Pinned signer verification and key transitions for catalog revisions.

use crate::buckets::rotation::CatalogKeyChange;
use crate::buckets::signature::{verify_catalog_directory, CatalogKeyId, CatalogPublicKey};
use crate::error::{coded, ErrorCode};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

const MAX_CATALOG_PUBLIC_KEY_BYTES: u64 = 4 * 1024;

/// Minisign signer pinned for one Git catalog at first sync.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct CatalogSigner {
    /// Minisign key identifier shown as 16 uppercase hexadecimal digits.
    pub key_id: String,
    /// The minisign public key text accepted at first sync.
    pub public_key: String,
    /// Pinned offline revocation key identifier, when the catalog named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revocation_key_id: Option<String>,
    /// The minisign revocation public key text accepted at first sync.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revocation_public_key: Option<String>,
    /// Key identifiers that this catalog rotated away from, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub previous_signers: Vec<String>,
    /// Key identifiers that the offline revocation key revoked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub revoked_signers: Vec<String>,
    /// BLAKE3 of the rotation statement already applied to this pin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_rotation: Option<String>,
    /// BLAKE3 of the revocation statement already applied to this pin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_revocation: Option<String>,
    /// BLAKE3 digests of every key statement this pin applied, oldest first.
    ///
    /// The history makes a statement single-use: a revision that republishes an
    /// applied statement cannot rotate the pin back to the key it named.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub applied_key_statements: Vec<String>,
}

impl CatalogSigner {
    pub(super) fn pin(
        key: &CatalogPublicKey,
        text: String,
        revocation: Option<(CatalogPublicKey, String)>,
    ) -> Self {
        let (revocation_key_id, revocation_public_key) = match revocation {
            Some((key, text)) => (Some(key.key_id().to_string()), Some(text)),
            None => (None, None),
        };
        Self {
            key_id: key.key_id().to_string(),
            public_key: text,
            revocation_key_id,
            revocation_public_key,
            previous_signers: Vec::new(),
            revoked_signers: Vec::new(),
            applied_rotation: None,
            applied_revocation: None,
            applied_key_statements: Vec::new(),
        }
    }

    pub(super) fn pinned_key(&self) -> Result<CatalogPublicKey> {
        let key = CatalogPublicKey::parse(&self.public_key)
            .context("pinned catalog signer is invalid")?;
        if key.key_id().to_string() != self.key_id {
            bail!("pinned catalog signer key ID does not match its public key");
        }
        Ok(key)
    }

    pub(super) fn pinned_revocation_key(&self) -> Result<Option<CatalogPublicKey>> {
        let (Some(key_id), Some(text)) = (&self.revocation_key_id, &self.revocation_public_key)
        else {
            return Ok(None);
        };
        let key = CatalogPublicKey::parse(text).context("pinned revocation key is invalid")?;
        if key.key_id().to_string() != *key_id {
            bail!("pinned revocation key ID does not match its public key");
        }
        Ok(Some(key))
    }

    fn rotated_to(
        &self,
        change: &crate::buckets::rotation::VerifiedKeyChange,
        text: String,
    ) -> Self {
        let mut signer = self.clone();
        signer.forget_retired(&change.to);
        signer.forget_retired(&change.from);
        signer.previous_signers.push(change.from.clone());
        signer.key_id = change.to.clone();
        signer.public_key = text;
        signer.applied_rotation = Some(change.body_digest.clone());
        signer
            .applied_key_statements
            .push(change.body_digest.clone());
        signer.assert_pinned_not_retired();
        signer
    }

    fn revoked_to(
        &self,
        change: &crate::buckets::rotation::VerifiedKeyChange,
        text: String,
    ) -> Self {
        let mut signer = self.clone();
        signer.forget_retired(&change.to);
        signer.forget_retired(&change.from);
        signer.revoked_signers.push(change.from.clone());
        signer.key_id = change.to.clone();
        signer.public_key = text;
        signer.applied_revocation = Some(change.body_digest.clone());
        signer
            .applied_key_statements
            .push(change.body_digest.clone());
        signer.assert_pinned_not_retired();
        signer
    }

    /// Drop one key identifier from both retired-key lists.
    ///
    /// A rotation or revocation statement that names an earlier key as its
    /// successor re-authorizes that key: it is current again, not retired.
    fn forget_retired(&mut self, key_id: &str) {
        self.previous_signers
            .retain(|retired| !retired.eq_ignore_ascii_case(key_id));
        self.revoked_signers
            .retain(|retired| !retired.eq_ignore_ascii_case(key_id));
    }

    fn is_retired(&self, key_id: &str) -> bool {
        self.previous_signers
            .iter()
            .chain(self.revoked_signers.iter())
            .any(|retired| retired.eq_ignore_ascii_case(key_id))
    }

    /// The current signer must never appear as retired.
    ///
    /// A pin whose current key is listed as retired refuses its own revisions,
    /// so the rule is enforced here and checked by tests.
    fn assert_pinned_not_retired(&self) {
        debug_assert!(
            !self.is_retired(&self.key_id),
            "the pinned signing key must not be a retired key"
        );
    }
}

fn unsigned(label: &str) -> anyhow::Error {
    anyhow!("{label} is unsigned; Modn refuses unsigned catalogs.")
}

fn has_signature(root: &Path) -> Result<bool> {
    match fs::symlink_metadata(root.join("catalog.sig")) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn read_public_key(root: &Path) -> Result<Option<(CatalogPublicKey, String)>> {
    let path = root.join("catalog.pub");
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.file_type().is_file() {
        bail!(
            "catalog public key must be a regular file: {}",
            path.display()
        );
    }
    if metadata.len() > MAX_CATALOG_PUBLIC_KEY_BYTES {
        bail!("catalog public key exceeds size limit: {}", path.display());
    }
    let text = fs::read_to_string(&path)
        .with_context(|| format!("cannot read catalog public key: {}", path.display()))?;
    let key = CatalogPublicKey::parse(&text)?;
    Ok(Some((key, text)))
}

/// Result of verifying one staged catalog revision.
pub(in crate::buckets) struct CatalogVerification {
    /// The signer pin to publish.
    pub(in crate::buckets) signer: CatalogSigner,
    /// True when this sync pinned the signer key for the first time.
    pub(in crate::buckets) pinned: bool,
    /// An accepted rotation or revocation, when this revision carries one.
    pub(in crate::buckets) key_change: Option<CatalogKeyChange>,
    /// Published key statements that added no trust to this pin.
    pub(in crate::buckets) ignored_key_statements: Vec<String>,
}

/// Verify one staged catalog tree and return the signer to publish.
///
/// The first sync pins the key from `catalog.pub` and the optional key from
/// `revocation.pub`. A later sync verifies with the pinned keys only, accepts a
/// signed rotation or revocation statement, and refuses any other key change.
pub(in crate::buckets) fn verify_catalog_tree(
    root: &Path,
    name: &str,
    label: &str,
    previous: Option<&CatalogSigner>,
    expected_fingerprint: Option<&CatalogKeyId>,
) -> Result<CatalogVerification> {
    if !has_signature(root)? {
        return Err(unsigned(label));
    }
    let revision_public = read_public_key(root)?;
    if let (Some(expected), Some((key, _))) = (expected_fingerprint, revision_public.as_ref()) {
        let actual = key.key_id();
        if actual != *expected {
            return Err(coded(
                ErrorCode::BucketFingerprintMismatch,
                format!(
                    "Catalog for known bucket {name} publishes signer key {actual}, but the official main bucket lists {expected}; the fingerprint mismatch refused the sync and nothing was written."
                ),
            ));
        }
    }
    let revocation_public = crate::buckets::rotation::read_revocation_public_key(root)?;
    let Some(previous) = previous else {
        let Some((key, text)) = revision_public else {
            return Err(unsigned(label));
        };
        let ignored = crate::buckets::rotation::validate_historical_statements(root, name)?;
        verify_catalog_directory(root, &key)?;
        return Ok(CatalogVerification {
            signer: CatalogSigner::pin(&key, text, revocation_public),
            pinned: true,
            key_change: None,
            ignored_key_statements: ignored,
        });
    };
    let pinned = previous.pinned_key()?;
    let pinned_revocation = previous.pinned_revocation_key()?;
    crate::buckets::rotation::check_revocation_key(
        name,
        pinned_revocation.as_ref(),
        revocation_public.as_ref().map(|(key, _)| key),
    )?;
    let catalog_name = crate::buckets::signature::catalog_name(root)?;
    let mut applied: Vec<&str> = previous
        .applied_key_statements
        .iter()
        .map(String::as_str)
        .collect();
    for digest in [
        previous.applied_rotation.as_deref(),
        previous.applied_revocation.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if !applied.contains(&digest) {
            applied.push(digest);
        }
    }
    let keys = crate::buckets::rotation::PinnedKeys {
        signer: &pinned,
        revocation: pinned_revocation.as_ref(),
        applied,
    };
    let outcome = crate::buckets::rotation::apply_key_statements(
        root,
        &catalog_name,
        &keys,
        revision_public.as_ref().map(|(key, _)| key),
    )?;
    let Some(change) = outcome.change else {
        if let Some((present, _)) = &revision_public {
            refuse_retired_signer(name, previous, present)?;
        }
        verify_catalog_directory(root, &pinned)?;
        return Ok(CatalogVerification {
            signer: previous.clone(),
            pinned: false,
            key_change: None,
            ignored_key_statements: outcome.ignored,
        });
    };
    verify_catalog_directory(root, &change.new_key)?;
    let signer = match change.kind {
        crate::buckets::rotation::KeyChangeKind::Rotated => {
            if previous
                .revoked_signers
                .iter()
                .any(|revoked| revoked.eq_ignore_ascii_case(&change.to))
            {
                bail!(
                    "Rotation statement for bucket {name} names key {}, which was revoked by the offline revocation key; only the offline revocation key can re-authorize a revoked key.",
                    change.to
                );
            }
            previous.rotated_to(&change, change.new_key_text.clone())
        }
        crate::buckets::rotation::KeyChangeKind::Revoked => {
            previous.revoked_to(&change, change.new_key_text.clone())
        }
    };
    Ok(CatalogVerification {
        signer,
        pinned: false,
        key_change: Some(change.report()),
        ignored_key_statements: outcome.ignored,
    })
}

fn refuse_retired_signer(
    name: &str,
    previous: &CatalogSigner,
    present: &CatalogPublicKey,
) -> Result<()> {
    let fingerprint = present.key_id().to_string();
    if previous
        .revoked_signers
        .iter()
        .any(|revoked| revoked.eq_ignore_ascii_case(&fingerprint))
    {
        bail!(
            "Signing key {fingerprint} for bucket {name} was revoked by the offline revocation key; publish a revision signed by the replacement key."
        );
    }
    if previous
        .previous_signers
        .iter()
        .any(|old| old.eq_ignore_ascii_case(&fingerprint))
    {
        bail!(
            "Signing key {fingerprint} for bucket {name} was rotated away; publish a revision signed by the current key."
        );
    }
    let pinned = previous.pinned_key()?;
    if *present != pinned {
        if present.key_id().to_string() == previous.key_id {
            bail!(
                "Signer key {} for bucket {name} publishes a different public key than the pinned key. A key change needs a signed rotation or revocation statement; remove the bucket and sync it again to trust the new key.",
                previous.key_id
            );
        }
        bail!(
            "Signer key changed from {} to {fingerprint} for bucket {name}. A key change needs a signed rotation or revocation statement; remove the bucket and sync it again to trust the new key.",
            pinned.key_id()
        );
    }
    Ok(())
}
