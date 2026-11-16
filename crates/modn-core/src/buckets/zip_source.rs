//! Signed catalog ZIP sources fetched from HTTPS or an absolute local path.
//!
//! The ZIP root holds the same files as a Git catalog. Modn inspects the
//! archive, extracts it into staging, verifies the signature and the contract
//! v2 layout, then publishes the catalog and updates one registration.
use super::git::{validate_tree, verify_catalog_tree, CatalogSigner};
use super::lock::{acquire_lock, LOCK_WAIT};
use super::manifest::valid_id;
use super::mirrors::{
    check_mirror_kinds, read_mirrors, read_relocation, CatalogHost, CatalogSyncTrace,
};
use super::rotation::{CatalogKeyChange, KeyChangeKind};
use super::store::{bounded_json, BucketStore, Registration};
use super::trust::ModuleTrustState;
use crate::game_modules::{ignored_catalog_modules, stage_catalog_modules_at, CatalogModuleReport};
use crate::{archive, bucket_digest};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
/// Largest accepted catalog ZIP body.
pub const MAX_CATALOG_ZIP_BYTES: u64 = 256 * 1024 * 1024;
/// Largest accepted uncompressed size of one catalog entry.
pub const MAX_CATALOG_ENTRY_BYTES: u64 = 64 * 1024 * 1024;
/// Largest accepted total uncompressed size of a catalog ZIP.
pub const MAX_CATALOG_UNCOMPRESSED_BYTES: u64 = 1024 * 1024 * 1024;
/// Largest accepted uncompressed-to-compressed ratio of one catalog entry.
pub const MAX_CATALOG_EXPANSION_RATIO: u64 = 200;
/// An explicitly selected community catalog ZIP and its location.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ZipCatalogSource {
    /// HTTPS URL or absolute local ZIP path. Local paths support offline fixtures.
    pub url: String,
}
impl ZipCatalogSource {
    /// Construct a source. Sync validates it before any network call.
    #[must_use]
    pub fn new(url: String) -> Self {
        Self { url }
    }
    /// True when a `--source` value selects the ZIP kind.
    #[must_use]
    pub fn selects(text: &str) -> bool {
        text.to_ascii_lowercase().ends_with(".zip")
    }
    /// Validate this source before any network call.
    pub(super) fn validate(&self) -> Result<()> {
        if self.url.starts_with("http://") {
            bail!("catalog ZIP source must use https://; plain HTTP is refused");
        }
        let local = PathBuf::from(&self.url);
        let https = self.url.strip_prefix("https://").is_some_and(|rest| {
            let host = rest.split('/').next().unwrap_or_default();
            !host.is_empty() && !host.contains(['@', '?', '#', '\\', ':'])
        });
        if (!https && !local.is_absolute())
            || self.url.chars().any(char::is_control)
            || (https && self.url.contains('\\'))
            || !Self::selects(&self.url)
        {
            bail!("catalog ZIP source must be an HTTPS URL or an absolute local ZIP file path");
        }
        Ok(())
    }
    fn label(&self) -> &str {
        &self.url
    }
}
/// Source, verified digest and signer published with one ZIP catalog.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ZipCatalogRevision {
    /// Explicit source retained for subsequent updates.
    pub source: ZipCatalogSource,
    /// Canonical catalog digest of the published tree.
    pub digest: String,
    /// Signer key pinned at first sync.
    pub signer: CatalogSigner,
}
/// Result of one explicit ZIP catalog sync.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct ZipCatalogSync {
    /// The published source, digest and signer pin.
    #[serde(flatten)]
    pub revision: ZipCatalogRevision,
    /// True when this sync pinned the signer key.
    pub pinned: bool,
    /// Accepted rotation or revocation, when this revision carried one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_change: Option<CatalogKeyChange>,
    /// Published key statements that added no trust to this pin.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ignored_key_statements: Vec<String>,
    /// Game module releases imported or ignored during this sync.
    pub modules: CatalogModuleReport,
    /// The primary, the source that served the revision and any failures.
    pub trace: CatalogSyncTrace,
}
/// Stored ZIP revision. The signer is always present for ZIP catalogs.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredZipCatalogRevision {
    pub(super) source: ZipCatalogSource,
    pub(super) digest: String,
    pub(super) signer: CatalogSigner,
}
impl StoredZipCatalogRevision {
    pub(super) fn revision(self) -> ZipCatalogRevision {
        ZipCatalogRevision {
            source: self.source,
            digest: self.digest,
            signer: self.signer,
        }
    }
}
impl From<ZipCatalogRevision> for StoredZipCatalogRevision {
    fn from(revision: ZipCatalogRevision) -> Self {
        Self {
            source: revision.source,
            digest: revision.digest,
            signer: revision.signer,
        }
    }
}
fn download_to(path: &Path, mut reader: impl Read) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let copied = io::copy(
        &mut reader.by_ref().take(MAX_CATALOG_ZIP_BYTES + 1),
        &mut file,
    )?;
    if copied > MAX_CATALOG_ZIP_BYTES {
        bail!("catalog ZIP download exceeds the 256 MiB size limit");
    }
    file.sync_all()?;
    Ok(())
}
fn check_entry_limits(path: &Path) -> Result<()> {
    let inspection = archive::inspect(path)?;
    if inspection.total_uncompressed_bytes > MAX_CATALOG_UNCOMPRESSED_BYTES {
        bail!("catalog ZIP exceeds the 1 GiB uncompressed size limit");
    }
    let mut zip = zip::ZipArchive::new(File::open(path)?).context("invalid ZIP catalog")?;
    for file in &inspection.files {
        if file.bytes > MAX_CATALOG_ENTRY_BYTES {
            bail!(
                "catalog ZIP entry exceeds the 64 MiB size limit: {}",
                file.entry
            );
        }
        let name = file.original_entry.as_deref().unwrap_or(&file.entry);
        if name.contains('\\') {
            bail!("catalog ZIP entry uses a backslash separator: {name:?}");
        }
        let entry = zip
            .by_name(name)
            .with_context(|| format!("cannot read catalog ZIP entry {name}"))?;
        let compressed = entry.compressed_size();
        let ratio = file.bytes / compressed.max(1);
        if file.bytes > 0 && ratio > MAX_CATALOG_EXPANSION_RATIO {
            bail!(
                "catalog ZIP entry exceeds the expansion ratio limit: {}",
                file.entry
            );
        }
    }
    Ok(())
}
fn extract_catalog(path: &Path, stage: &Path) -> Result<()> {
    check_entry_limits(path)?;
    let inspection = archive::inspect(path)?;
    fs::create_dir(stage)?;
    let mut zip = zip::ZipArchive::new(File::open(path)?).context("invalid ZIP catalog")?;
    for file in inspection.files {
        let target = stage.join(&file.entry);
        fs::create_dir_all(target.parent().context("missing catalog entry parent")?)?;
        let mut input = zip.by_name(file.original_entry.as_deref().unwrap_or(&file.entry))?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)?;
        let copied = io::copy(&mut input.by_ref().take(file.bytes + 1), &mut output)?;
        if copied != file.bytes {
            bail!("catalog ZIP extraction size mismatch: {}", file.entry);
        }
        output.sync_all()?;
    }
    Ok(())
}
impl BucketStore {
    /// Pinned signer of one registered catalog, whatever its source kind.
    pub fn catalog_signer(&self, name: &str) -> Result<Option<super::CatalogSigner>> {
        if !valid_id(name) || name == "main" {
            bail!("invalid or reserved bucket name");
        }
        let registration = self.registration(name)?;
        Ok(registration.catalog_signer().cloned())
    }
    /// Read the published ZIP revision without fetching.
    pub fn zip_revision(&self, name: &str) -> Result<Option<ZipCatalogRevision>> {
        if !valid_id(name) || name == "main" {
            bail!("invalid or reserved ZIP catalog name");
        }
        self.registered_zip(name)
    }
    /// Download, extract, verify and validate a community catalog ZIP before publication.
    /// New catalogs require `source`; updates reuse the registered source when it is absent.
    /// A failed download, verification or validation leaves the old registration and tree intact.
    pub fn sync_zip_with_signer(
        &self,
        name: &str,
        source: Option<ZipCatalogSource>,
        mut open: impl FnMut(&ZipCatalogSource) -> Result<Box<dyn Read>>,
    ) -> Result<ZipCatalogSync> {
        self.check_registry_root()?;
        if !valid_id(name) || name == "main" {
            bail!("invalid or reserved ZIP catalog name");
        }
        fs::create_dir_all(&self.root)?;
        let lock_path = self.root.join(".git-sync.lock");
        let _lock = acquire_lock(
            &lock_path,
            "unsafe catalog lock",
            "catalog sync already running",
            LOCK_WAIT,
        )?;
        let previous = self.zip_revision(name)?;
        if previous.is_none() && self.registrations()?.contains_key(name) {
            bail!(
                "bucket {name} is not a ZIP catalog; remove it and sync it again to change its source kind"
            );
        }
        let trust = match &previous {
            None => ModuleTrustState::Untrusted,
            Some(_) => self.module_trust(name)?,
        };
        let source = match (source, previous.as_ref()) {
            (Some(source), None) => source,
            (None, Some(previous)) => previous.source.clone(),
            (Some(source), Some(previous)) if source == previous.source => source,
            _ => {
                bail!("provide a source for registration; changing a registered source is refused")
            }
        };
        source.validate()?;
        let primary = CatalogHost::from(&source);
        let mut candidates = vec![source.clone()];
        if previous.is_some() {
            for mirror in self.stored_mirrors(name)? {
                let Some(candidate) = mirror.zip_source() else {
                    continue;
                };
                if !candidates.contains(&candidate) {
                    candidates.push(candidate);
                }
            }
        }
        let stage = tempfile::Builder::new()
            .prefix("zip-catalog-")
            .tempdir_in(&self.root)?;
        let mut failed_sources = Vec::new();
        let mut served = None;
        for (index, candidate) in candidates.iter().enumerate() {
            let attempt = stage.path().join(format!("source-{index}"));
            fs::create_dir_all(&attempt)?;
            let zip_path = attempt.join("catalog.zip");
            let catalog = attempt.join("catalog");
            let fetched = open(candidate)
                .and_then(|reader| download_to(&zip_path, reader))
                .and_then(|()| extract_catalog(&zip_path, &catalog));
            match fetched {
                Ok(()) => {
                    fs::remove_file(&zip_path).ok();
                    served = Some((catalog, candidate.clone()));
                    break;
                }
                Err(error) => {
                    let _ = fs::remove_dir_all(&attempt);
                    failed_sources.push(format!(
                        "{}: {error:#}",
                        CatalogHost::from(candidate).label()
                    ));
                }
            }
        }
        let Some((catalog, served_source)) = served else {
            bail!(
                "catalog {name} could not be fetched from any source:\n{}",
                failed_sources.join("\n")
            );
        };
        let served_by = CatalogHost::from(&served_source);
        let label = format!("ZIP catalog {name} at {}", served_source.label());
        let verification = verify_catalog_tree(
            &catalog,
            name,
            &label,
            previous.as_ref().map(|revision| &revision.signer),
        )
        .map_err(|error| anyhow!("catalog source {} failed: {error:#}", served_by.label()))?;
        let trust = match verification.key_change.as_ref().map(|change| change.kind) {
            Some(KeyChangeKind::Revoked) => ModuleTrustState::Untrusted,
            _ => trust,
        };
        let pinned = verification.pinned;
        validate_tree(&catalog, &catalog, trust)
            .map_err(|error| anyhow!("catalog source {} failed: {error:#}", served_by.label()))?;
        let mirrors = read_mirrors(&catalog)
            .map_err(|error| anyhow!("catalog source {} failed: {error:#}", served_by.label()))?;
        check_mirror_kinds(name, &served_by, &mirrors)
            .map_err(|error| anyhow!("catalog source {} failed: {error:#}", served_by.label()))?;
        let relocation = read_relocation(&catalog, &served_by)
            .map_err(|error| anyhow!("catalog source {} failed: {error:#}", served_by.label()))?;
        let moved_to = relocation.filter(|primary_host| primary_host != &primary);
        let published_source = match &moved_to {
            Some(host) => host
                .zip_source()
                .context("relocation must name a ZIP source")?,
            None => source,
        };
        let revision = ZipCatalogRevision {
            source: published_source,
            digest: bucket_digest::catalog_digest(&catalog)?.to_string(),
            signer: verification.signer,
        };
        let key_change = verification.key_change;
        let previous_registration = match &previous {
            Some(_) => Some(self.registration(name)?),
            None => None,
        };
        let unchanged = previous
            .as_ref()
            .is_some_and(|previous| previous.digest == revision.digest);
        let published_path = match (&previous_registration, unchanged) {
            (Some(registration), true) => registration.path.clone(),
            _ => catalog.clone(),
        };
        let superseded = match (&previous_registration, unchanged) {
            (Some(registration), false) => Some(registration.path.clone()),
            _ => None,
        };
        let registration_before = self.stored_registration_bytes(name)?;
        let staged = if trust == ModuleTrustState::Trusted {
            Some(stage_catalog_modules_at(
                &crate::data_dir::profile_directory()?,
                name,
                &revision.signer.key_id,
                &catalog,
            )?)
        } else {
            None
        };
        let ignored = match &staged {
            Some(staged) => staged.ignored().to_vec(),
            None => ignored_catalog_modules(&catalog)?,
        };
        self.publish_zip(
            name,
            published_path,
            revision.clone(),
            mirrors.clone(),
            trust == ModuleTrustState::Trusted,
        )?;
        let retained = if unchanged && previous_registration.is_some() {
            None
        } else {
            Some(stage.keep())
        };
        let imported = match staged {
            Some(staged) => match staged.commit() {
                Ok(imported) => imported,
                Err(error) => {
                    let restored = self.restore_registration(name, registration_before.as_deref());
                    if let Some(retained) = &retained {
                        let _ = fs::remove_dir_all(retained);
                    }
                    return match restored {
                        Ok(()) => Err(error),
                        Err(restore_error) => Err(error.context(format!(
                            "the previous registration could not be restored: {restore_error:#}"
                        ))),
                    };
                }
            },
            None => Vec::new(),
        };
        if let Some(superseded) = superseded {
            self.retire_catalog_snapshot(&superseded);
        }
        Ok(ZipCatalogSync {
            revision,
            pinned,
            key_change,
            ignored_key_statements: verification.ignored_key_statements,
            modules: CatalogModuleReport { imported, ignored },
            trace: CatalogSyncTrace {
                primary: primary.clone(),
                served_by,
                failed_sources,
                moved_from: moved_to.as_ref().map(|_| primary),
                moved_to,
            },
        })
    }
    pub(super) fn publish_zip(
        &self,
        name: &str,
        path: PathBuf,
        zip: super::ZipCatalogRevision,
        mirrors: Vec<CatalogHost>,
        trusted: bool,
    ) -> Result<()> {
        let signer_key_id = zip.signer.key_id.clone();
        let module_trust = trusted.then_some(super::ModuleSourceTrust { signer_key_id });
        let trust = if module_trust.is_some() {
            ModuleTrustState::Trusted
        } else {
            ModuleTrustState::Untrusted
        };
        self.validate_catalog(name, &path, trust)?;
        self.write_registration(
            name,
            &Registration {
                path,
                git: None,
                zip: Some(zip.into()),
                module_trust,
                mirrors: Some(mirrors),
            },
        )
    }
    pub(super) fn registered_zip(&self, name: &str) -> Result<Option<super::ZipCatalogRevision>> {
        if !self.registrations()?.contains_key(name) {
            return Ok(None);
        }
        let registration: Registration =
            bounded_json(&self.root.join("registrations").join(format!("{name}.json")))?;
        Ok(registration.zip.map(|stored| stored.revision()))
    }
}
#[cfg(test)]
mod key_tests;
#[cfg(test)]
mod snapshot_tests;
#[cfg(test)]
mod tests;
