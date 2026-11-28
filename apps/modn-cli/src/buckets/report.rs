//! Sequence and expiry lines that catalog sync and refresh print.
//!
//! The accepted number lives in the signed catalog tree, so the report reads
//! the installed copy that the sync just published. The expiry warning comes
//! from the core sync result; the line names the signed expiry when the tree
//! still carries it.

use crate::catalog::sequence::{self, SequenceFile};
use modn_core::buckets::{CatalogWarning, CatalogWarningKind};
use modn_core::manager_roots::ManagerRoots;
use serde_json::Value;
use std::path::PathBuf;

/// What one accepted catalog reported about its signed number and expiry.
#[derive(Clone, Debug, Default)]
pub(crate) struct CatalogReport {
    /// Accepted signed number, when the catalog is numbered.
    pub(crate) sequence: Option<u64>,
    /// Warning records for additive JSON output.
    pub(crate) warnings: Vec<CatalogWarning>,
    /// Text lines for command output.
    pub(crate) lines: Vec<String>,
}

/// Read the accepted number of one catalog and word its report.
pub(crate) fn catalog(
    roots: &ManagerRoots,
    name: &str,
    warnings: &[CatalogWarning],
) -> CatalogReport {
    let sequence = accepted(roots, name);
    let number = sequence.as_ref().map(|file| file.sequence);
    let mut lines = Vec::new();
    if let Some(number) = number {
        lines.push(format!("sequence {number} accepted"));
    }
    for warning in warnings {
        lines.push(warning_line(name, sequence.as_ref(), warning));
    }
    CatalogReport {
        sequence: number,
        warnings: warnings.to_vec(),
        lines,
    }
}

fn warning_line(name: &str, sequence: Option<&SequenceFile>, warning: &CatalogWarning) -> String {
    if warning.kind == CatalogWarningKind::Expired {
        let expires = sequence
            .and_then(|file| file.expires.as_deref())
            .unwrap_or("its signed expiry");
        return format!(
            "warning: catalog {name} expired on {expires}; its publisher asks users not to rely on it. The sync completed."
        );
    }
    format!("warning: {}", warning.message)
}

fn accepted(roots: &ManagerRoots, name: &str) -> Option<SequenceFile> {
    let root = if name == "main" {
        roots.official_main()
    } else {
        registered_tree(roots, name)?
    };
    sequence::read(&root).ok().flatten()
}

fn registered_tree(roots: &ManagerRoots, name: &str) -> Option<PathBuf> {
    let path = roots
        .bucket_registrations()
        .join("registrations")
        .join(format!("{name}.json"));
    let bytes = std::fs::read(path).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    Some(PathBuf::from(value.get("path")?.as_str()?))
}
