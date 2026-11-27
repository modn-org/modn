//! Signed sequence plumbing shared by the catalog sync paths.

use super::super::sequence::StoredSequence;
use super::super::signature::CatalogKeyId;
use super::verify::{verify_catalog_tree, CatalogVerification};
use super::GitCatalogRevision;
use anyhow::{anyhow, Result};
use std::path::Path;

/// Add the failing source to a staged catalog error without losing its code.
///
/// The text keeps the flattened form that users already see. A fingerprint
/// mismatch keeps [`crate::error::ErrorCode::BucketFingerprintMismatch`], and a
/// signed sequence refusal keeps its own code.
pub(crate) fn catalog_source_error(label: &str, error: anyhow::Error) -> anyhow::Error {
    let message = format!("catalog source {label} failed: {error:#}");
    match crate::error::code_of(&error) {
        Some(
            code @ (crate::error::ErrorCode::BucketFingerprintMismatch
            | crate::error::ErrorCode::CatalogRollback
            | crate::error::ErrorCode::CatalogSequenceConflict),
        ) => crate::error::coded(code, message),
        _ => anyhow!(message),
    }
}

pub(super) fn verify_revision(
    root: &Path,
    name: &str,
    reference: &str,
    previous: Option<&GitCatalogRevision>,
    expected_fingerprint: Option<&CatalogKeyId>,
    stored_sequence: Option<&StoredSequence>,
    now: u64,
) -> Result<CatalogVerification> {
    verify_catalog_tree(
        root,
        name,
        &format!("Git catalog {name} at {reference}"),
        previous.map(|revision| &revision.signer),
        expected_fingerprint,
        stored_sequence,
        now,
    )
}
