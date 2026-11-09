//! Module-source trust records for registered buckets.

use super::lock::{acquire_lock, LOCK_WAIT};
use super::manifest::valid_id;
use super::signature::{verify_catalog_directory, CatalogPublicKey};
use super::store::Registration;
use super::BucketStore;
use crate::game_modules::ImportReport;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Stored acknowledgement that one bucket may supply executable game modules.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ModuleSourceTrust {
    /// Pinned Git catalog signer key identifier accepted for module code.
    pub signer_key_id: String,
}

/// Trust state of one bucket as a module source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ModuleTrustState {
    /// The official main bucket. Always a trusted module source.
    Main,
    /// A signed Git catalog whose trust record matches its pinned signer.
    Trusted,
    /// A registered bucket that is not a trusted module source.
    Untrusted,
    /// A local folder bucket. It can never supply modules.
    Local,
}

/// User acknowledgement required before modules from a bucket are trusted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "Callers must state which acknowledgement they supply"
)]
pub enum ModuleTrustAck<'a> {
    /// The user did not accept executable module code. No record is written.
    NotAcknowledged,
    /// The user accepted executable module code from this bucket.
    Acknowledged {
        /// Optional fingerprint that must match the pinned signer key ID.
        expected_fingerprint: Option<&'a str>,
    },
}

/// Result of trusting one bucket as a module source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct TrustModulesOutcome {
    /// Registered bucket name.
    pub bucket: String,
    /// Pinned signer key ID stored in the trust record.
    pub signer_key_id: String,
    /// Module releases imported from the bucket during this trust operation.
    pub imported: Vec<ImportReport>,
}

/// One bucket currently trusted as a module source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct TrustedModuleSource {
    /// Registered bucket name.
    pub bucket: String,
    /// Pinned signer key ID accepted for module code.
    pub signer_key_id: String,
}

impl BucketStore {
    /// Mark one signed Git catalog as a trusted module source.
    ///
    /// The caller must acknowledge executable module code. The stored record is
    /// tied to the pinned signer key ID, so a later signer key change ends trust.
    pub fn trust_modules(
        &self,
        name: &str,
        ack: ModuleTrustAck<'_>,
    ) -> Result<TrustModulesOutcome> {
        if name == "main" {
            bail!("the official main bucket is always a module source and needs no trust record");
        }
        if !valid_id(name) {
            bail!("invalid bucket name");
        }
        self.registration(name)?;
        fs::create_dir_all(&self.root)?;
        let _lock = acquire_lock(
            &self.root.join(".git-sync.lock"),
            "unsafe bucket lock",
            "bucket registration is already being written",
            LOCK_WAIT,
        )?;
        let registration = self.registration(name)?;
        let Some(revision) = registration.revision(name)? else {
            bail!("local folder buckets cannot be module sources: {name}");
        };
        let key_id = revision.signer.key_id;
        match ack {
            ModuleTrustAck::NotAcknowledged => bail!(
                "trusting modules from bucket {name} runs publisher code; acknowledgement is required for signer {key_id}"
            ),
            ModuleTrustAck::Acknowledged { expected_fingerprint } => {
                if let Some(expected) = expected_fingerprint {
                    if expected != key_id {
                        bail!(
                            "signer fingerprint mismatch for bucket {name}: expected {expected}, pinned signer is {key_id}"
                        );
                    }
                }
            }
        }
        let key = CatalogPublicKey::parse(&revision.signer.public_key)
            .context("pinned catalog signer is invalid")?;
        if key.key_id().to_string() != key_id {
            bail!("pinned catalog signer key ID does not match its public key");
        }
        verify_catalog_directory(&registration.path, &key).with_context(|| {
            format!(
                "cannot trust modules from bucket {name}; the published catalog does not verify"
            )
        })?;
        let staged = crate::game_modules::stage_catalog_modules_at(
            self.data_root(),
            name,
            &key_id,
            &registration.path,
        )
        .with_context(|| {
            format!("cannot import game modules from bucket {name}; nothing was trusted")
        })?;
        self.write_registration(
            name,
            &Registration {
                module_trust: Some(ModuleSourceTrust {
                    signer_key_id: key_id.clone(),
                }),
                ..registration
            },
        )?;
        let imported = staged
            .commit()
            .context("bucket modules were trusted but their releases could not be activated")?;
        Ok(TrustModulesOutcome {
            bucket: name.to_owned(),
            signer_key_id: key_id,
            imported,
        })
    }

    /// Remove a bucket's module-trust record. The bucket stays registered.
    pub fn untrust_modules(&self, name: &str) -> Result<()> {
        if name == "main" {
            bail!("cannot untrust the official main bucket");
        }
        if !valid_id(name) {
            bail!("invalid bucket name");
        }
        self.registration(name)?;
        fs::create_dir_all(&self.root)?;
        let _lock = acquire_lock(
            &self.root.join(".git-sync.lock"),
            "unsafe bucket lock",
            "bucket registration is already being written",
            LOCK_WAIT,
        )?;
        let registration = self.registration(name)?;
        if registration.git.is_none() {
            bail!("local folder buckets cannot be module sources: {name}");
        }
        if registration.module_trust.is_none() {
            return Ok(());
        }
        self.write_registration(
            name,
            &Registration {
                module_trust: None,
                ..registration
            },
        )
    }

    /// Report whether a bucket may supply executable game modules.
    pub fn module_trust(&self, name: &str) -> Result<ModuleTrustState> {
        if name == "main" {
            return Ok(ModuleTrustState::Main);
        }
        if !valid_id(name) {
            bail!("invalid bucket name");
        }
        let registration = self.registration(name)?;
        let Some(git) = registration.git else {
            return Ok(ModuleTrustState::Local);
        };
        let Some(signer) = git.signer else {
            return Ok(ModuleTrustState::Untrusted);
        };
        let trusted = registration
            .module_trust
            .is_some_and(|trust| trust.signer_key_id == signer.key_id);
        Ok(if trusted {
            ModuleTrustState::Trusted
        } else {
            ModuleTrustState::Untrusted
        })
    }

    /// List trusted module catalogs with their published directory.
    pub(crate) fn trusted_module_catalogs(&self) -> Result<Vec<(String, PathBuf)>> {
        let mut catalogs = Vec::new();
        for (bucket, registration) in self.registrations()? {
            if Self::stored_module_trust(&registration) != ModuleTrustState::Trusted {
                continue;
            }
            catalogs.push((bucket, registration.path));
        }
        Ok(catalogs)
    }

    /// List each bucket whose trust record matches its pinned signer key ID.
    pub fn trusted_module_sources(&self) -> Result<Vec<TrustedModuleSource>> {
        let mut sources = Vec::new();
        for (bucket, registration) in self.registrations()? {
            let Some(trust) = registration.module_trust else {
                continue;
            };
            let Some(git) = registration.git else {
                continue;
            };
            let Some(signer) = git.signer else {
                continue;
            };
            if trust.signer_key_id == signer.key_id {
                sources.push(TrustedModuleSource {
                    bucket,
                    signer_key_id: signer.key_id,
                });
            }
        }
        Ok(sources)
    }
}
