//! Evaluation of published rotation and revocation statements against a pin.

use super::{
    key_id_text, key_statement_bytes, read_bounded, read_revocation_public_key, KeyChangeKind,
    KeyStatementOutcome, PinnedKeys, RevocationBody, RotationBody, VerifiedKeyChange,
    MAX_SIGNATURE_BYTES, MAX_STATEMENT_BYTES, REVOCATION_FILE, REVOCATION_SIGNATURE_FILE,
    ROTATION_FILE, ROTATION_SIGNATURE_FILE,
};
use crate::buckets::signature::{signature_key_id, CatalogPublicKey};
use anyhow::{anyhow, bail, Context, Result};
use std::path::Path;

struct Statement {
    kind: KeyChangeKind,
    old_key_id: String,
    new_key: CatalogPublicKey,
    new_key_text: String,
    sequence: Option<u64>,
    body: Vec<u8>,
    signature: String,
}

/// Evaluate each published statement against the pinned key.
///
/// A statement applies only when its `from` or `revoked` key identifier names
/// the pinned signing key and the matching pinned key signed it. Every other
/// statement is history and adds no trust. Two statements that both apply, or a
/// chain that needs more than one change, refuse the revision.
pub(in crate::buckets) fn apply_key_statements(
    root: &Path,
    catalog_name: &str,
    pinned: &PinnedKeys<'_>,
    revision_public: Option<&CatalogPublicKey>,
    revision_sequence: Option<u64>,
) -> Result<KeyStatementOutcome> {
    let mut statements = Vec::new();
    if let Some((body, signature)) = read_statement(root, ROTATION_FILE, ROTATION_SIGNATURE_FILE)? {
        statements.push(parse_rotation(catalog_name, body, signature)?);
    }
    if let Some((body, signature)) =
        read_statement(root, REVOCATION_FILE, REVOCATION_SIGNATURE_FILE)?
    {
        statements.push(parse_revocation(catalog_name, body, signature)?);
    }
    for statement in &statements {
        check_statement_not_newer(catalog_name, statement.sequence, revision_sequence)?;
    }
    let pinned_id = pinned.signer.key_id().to_string();
    let applied: Vec<usize> = statements
        .iter()
        .enumerate()
        .filter(|(_, statement)| {
            pinned
                .applied
                .iter()
                .any(|digest| *digest == body_digest(&statement.body))
        })
        .map(|(index, _)| index)
        .collect();
    let applies: Vec<usize> = statements
        .iter()
        .enumerate()
        .filter(|(index, statement)| {
            !applied.contains(index) && statement.old_key_id.eq_ignore_ascii_case(&pinned_id)
        })
        .map(|(index, _)| index)
        .collect();
    let ignored = statements
        .iter()
        .enumerate()
        .filter(|(index, _)| !applies.contains(index))
        .map(|(_, statement)| kind_name(statement.kind).to_owned())
        .collect();
    let [index] = applies.as_slice() else {
        if applies.is_empty() {
            // An applied statement may stay published as history while the
            // revision stays on the pinned key. It must not move the pin again.
            let replays_pin = revision_public.is_some_and(|key| key != pinned.signer);
            let replayed = replays_pin
                && statements.iter().enumerate().any(|(index, statement)| {
                    applied.contains(&index)
                        && statement.old_key_id.eq_ignore_ascii_case(&pinned_id)
                });
            if replayed {
                bail!(
                    "catalog {catalog_name} republishes a key statement that this pin already applied; a rotation or revocation statement is applied once, so a retired key cannot regain trust by replaying it"
                );
            }
            return Ok(KeyStatementOutcome {
                change: None,
                ignored,
            });
        }
        bail!(
            "catalog {catalog_name} contains both rotation.json and revocation.json that apply to the pinned signing key {pinned_id}; publish only one statement"
        )
    };
    let statement = &statements[*index];
    check_statement_is_current(catalog_name, statement.sequence, revision_sequence)?;
    verify_statement_signature(catalog_name, pinned, statement)?;
    if let Some(revision_public) = revision_public {
        if *revision_public != statement.new_key {
            let successor = statement.new_key.key_id().to_string();
            let chained = statements
                .iter()
                .any(|other| other.old_key_id.eq_ignore_ascii_case(&successor));
            if chained {
                bail!(
                    "catalog {catalog_name} needs more than one signing-key change from the pinned key {pinned_id}; sync an intermediate revision or remove the bucket and register it again"
                );
            }
        }
    }
    require_revision_key(
        catalog_name,
        kind_name(statement.kind),
        &statement.new_key,
        revision_public,
    )?;
    Ok(KeyStatementOutcome {
        change: Some(VerifiedKeyChange {
            kind: statement.kind,
            from: pinned_id,
            to: statement.new_key.key_id().to_string(),
            body_digest: body_digest(&statement.body),
            new_key: statement.new_key.clone(),
            new_key_text: statement.new_key_text.clone(),
        }),
        ignored,
    })
}

/// Validate every published statement at the first sync without applying it.
///
/// The first sync trusts `catalog.pub` on first use, so an old statement adds no
/// trust. Modn still refuses a statement with an unknown shape, kind, version or
/// domain, because an author who publishes one has made a mistake.
pub(in crate::buckets) fn validate_historical_statements(
    root: &Path,
    catalog_name: &str,
    revision_sequence: Option<u64>,
) -> Result<Vec<String>> {
    let mut ignored = Vec::new();
    if let Some((body, signature)) = read_statement(root, ROTATION_FILE, ROTATION_SIGNATURE_FILE)? {
        let statement = parse_rotation(catalog_name, body, signature)?;
        check_statement_not_newer(catalog_name, statement.sequence, revision_sequence)?;
        let signer = signature_key_id(&statement.signature, "rotation")?;
        if signer.to_string() != statement.old_key_id {
            bail!(
                "rotation statement for catalog {catalog_name} is signed by key {signer}, but its from field names {}; the statement is invalid",
                statement.old_key_id
            );
        }
        ignored.push(kind_name(statement.kind).to_owned());
    }
    if let Some((body, signature)) =
        read_statement(root, REVOCATION_FILE, REVOCATION_SIGNATURE_FILE)?
    {
        let statement = parse_revocation(catalog_name, body, signature)?;
        check_statement_not_newer(catalog_name, statement.sequence, revision_sequence)?;
        let Some((revocation_key, _)) = read_revocation_public_key(root)? else {
            bail!(
                "catalog {catalog_name} has a revocation statement, but no revocation.pub; the statement is invalid"
            );
        };
        let signer = signature_key_id(&statement.signature, "revocation")?;
        if signer != revocation_key.key_id() {
            bail!(
                "revocation statement for catalog {catalog_name} is signed by key {signer}, but revocation.pub names {}; the statement is invalid",
                revocation_key.key_id()
            );
        }
        ignored.push(kind_name(statement.kind).to_owned());
    }
    Ok(ignored)
}

/// Refuse a statement that names a number newer than the revision that carries it.
///
/// A statement can describe only the revision it was published in or an earlier
/// one. A higher number has no revision that could have carried it, so a
/// publisher who writes one has made a mistake.
fn check_statement_not_newer(
    catalog_name: &str,
    statement: Option<u64>,
    revision: Option<u64>,
) -> Result<()> {
    let Some(statement) = statement else {
        return Ok(());
    };
    match revision {
        Some(revision) if statement <= revision => Ok(()),
        Some(revision) => bail!(
            "a key statement in catalog {catalog_name} names sequence {statement}, which is newer than the catalog revision's sequence {revision}"
        ),
        None => Err(crate::error::coded(
            crate::error::ErrorCode::CatalogRollback,
            format!(
                "A key statement in catalog {catalog_name} names sequence {statement}, but the catalog revision is unnumbered. A numbered statement cannot appear in an unnumbered revision; the rollback refused the sync and nothing was written."
            ),
        )),
    }
}

/// Require the statement that applies to the pinned key to name this revision.
///
/// A statement that would move the pin and names an older number is a replay:
/// the revision it was published in is already superseded. Every other published
/// statement is history from an earlier revision. It adds no trust, and it keeps
/// the number it was signed with, so a lower number is normal there.
fn check_statement_is_current(
    catalog_name: &str,
    statement: Option<u64>,
    revision: Option<u64>,
) -> Result<()> {
    let (Some(statement), Some(revision)) = (statement, revision) else {
        return Ok(());
    };
    if statement < revision {
        return Err(crate::error::coded(
            crate::error::ErrorCode::CatalogRollback,
            format!(
                "A key statement in catalog {catalog_name} names sequence {statement}, but the catalog revision is sequence {revision}. A statement that applies to the pinned key must be published in the revision it names; a statement from an older revision is a replay, so the rollback refused the sync and nothing was written."
            ),
        ));
    }
    Ok(())
}

fn kind_name(kind: KeyChangeKind) -> &'static str {
    match kind {
        KeyChangeKind::Rotated => "rotation",
        KeyChangeKind::Revoked => "revocation",
    }
}

fn verify_statement_signature(
    catalog_name: &str,
    pinned: &PinnedKeys<'_>,
    statement: &Statement,
) -> Result<()> {
    let bytes = key_statement_bytes(statement.kind, catalog_name, &statement.body);
    match statement.kind {
        KeyChangeKind::Rotated => pinned
            .signer
            .verify_statement(&bytes, &statement.signature, "rotation")
            .with_context(|| {
                format!(
                    "rotation statement for catalog {catalog_name} is not signed by the pinned signing key {}",
                    pinned.signer.key_id()
                )
            }),
        KeyChangeKind::Revoked => {
            let Some(revocation_key) = pinned.revocation else {
                bail!(
                    "catalog {catalog_name} has a revocation statement, but no revocation key was pinned at first sync; Modn refuses it"
                );
            };
            revocation_key
                .verify_statement(&bytes, &statement.signature, "revocation")
                .with_context(|| {
                    format!(
                        "revocation statement for catalog {catalog_name} is not signed by the pinned revocation key {}",
                        revocation_key.key_id()
                    )
                })
        }
    }
}

fn parse_rotation(catalog_name: &str, body: Vec<u8>, signature: String) -> Result<Statement> {
    let statement: RotationBody = serde_json::from_slice(&body)
        .with_context(|| format!("invalid rotation statement for catalog {catalog_name}"))?;
    check_body(
        catalog_name,
        "rotation",
        statement.schema_version,
        &statement.kind,
    )?;
    let old_key_id = key_id_text(catalog_name, "rotation", "from", &statement.from)?;
    let new_key = CatalogPublicKey::parse(&statement.to).with_context(|| {
        format!("rotation statement for catalog {catalog_name} names an invalid key")
    })?;
    if old_key_id.eq_ignore_ascii_case(&new_key.key_id().to_string()) {
        bail!(
            "rotation statement for catalog {catalog_name} names key {old_key_id} as both from and to; it changes nothing"
        );
    }
    Ok(Statement {
        kind: KeyChangeKind::Rotated,
        old_key_id,
        new_key,
        new_key_text: statement.to,
        sequence: statement.sequence,
        body,
        signature,
    })
}

fn parse_revocation(catalog_name: &str, body: Vec<u8>, signature: String) -> Result<Statement> {
    let statement: RevocationBody = serde_json::from_slice(&body)
        .with_context(|| format!("invalid revocation statement for catalog {catalog_name}"))?;
    check_body(
        catalog_name,
        "revocation",
        statement.schema_version,
        &statement.kind,
    )?;
    let old_key_id = key_id_text(catalog_name, "revocation", "revoked", &statement.revoked)?;
    let new_key = CatalogPublicKey::parse(&statement.replacement).with_context(|| {
        format!("revocation statement for catalog {catalog_name} names an invalid replacement key")
    })?;
    if old_key_id.eq_ignore_ascii_case(&new_key.key_id().to_string()) {
        bail!(
            "revocation statement for catalog {catalog_name} names key {old_key_id} as both revoked and replacement; it changes nothing"
        );
    }
    Ok(Statement {
        kind: KeyChangeKind::Revoked,
        old_key_id,
        new_key,
        new_key_text: statement.replacement,
        sequence: statement.sequence,
        body,
        signature,
    })
}

fn body_digest(body: &[u8]) -> String {
    blake3::hash(body).to_hex().to_string()
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
    expected: &CatalogPublicKey,
    revision_public: Option<&CatalogPublicKey>,
) -> Result<()> {
    let Some(revision_public) = revision_public else {
        bail!(
            "{role} statement for catalog {catalog_name} names key {}, but this revision has no catalog.pub; a key change must publish the new public key",
            expected.key_id()
        );
    };
    if *revision_public != *expected {
        bail!(
            "{role} statement for catalog {catalog_name} names key {}, but this revision publishes a different public key with identifier {}; the full public key bytes must match",
            expected.key_id(),
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
