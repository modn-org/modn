//! Registration publication for a verified ZIP catalog.

use super::super::mirrors::CatalogHost;
use super::super::sequence::StoredSequence;
use super::super::store::{BucketStore, Registration};
use super::super::{ModuleSourceTrust, ModuleTrustState, ZipCatalogRevision};
use anyhow::Result;
use std::path::PathBuf;

impl BucketStore {
    pub(super) fn publish_zip(
        &self,
        name: &str,
        path: PathBuf,
        zip: ZipCatalogRevision,
        mirrors: Vec<CatalogHost>,
        trusted: bool,
        accepted: Option<StoredSequence>,
    ) -> Result<()> {
        let signer_key_id = zip.signer.key_id.clone();
        let module_trust = trusted.then_some(ModuleSourceTrust { signer_key_id });
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
                accepted,
            },
        )
    }
}
