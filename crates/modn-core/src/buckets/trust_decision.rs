//! One decision for whether a module release source may supply executable code.

use super::store::Registration;
use super::trust::{local_bucket_module_refusal, TrustedModuleSource};
use super::BucketStore;
use crate::game_modules::ModuleSource;
use anyhow::Result;
use std::fmt;

/// How one bucket was registered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ModuleSourceBucketKind {
    /// A signed Git or ZIP catalog.
    Catalog,
    /// A local folder bucket added with `modn bucket add`.
    LocalFolder,
}

/// Registration facts that decide whether a module release source is trusted.
///
/// The caller reads these from the registration of the bucket named by the
/// release source. [`decide_module_source_trust`] applies the owner rule and no
/// other input changes the outcome.
#[expect(
    clippy::exhaustive_structs,
    reason = "Registration facts are a closed set of inputs every caller must supply"
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleSourceRegistrationFacts {
    /// How the bucket was registered.
    pub kind: ModuleSourceBucketKind,
    /// Signer named by the bucket's module trust record, when one exists.
    pub trust_record_signer: Option<String>,
    /// Pinned catalog signer key identifier, when the bucket pins one.
    pub pinned_signer: Option<String>,
    /// Key identifiers the catalog rotated away from, oldest first.
    pub previous_signers: Vec<String>,
    /// Key identifiers the offline revocation key revoked.
    pub revoked_signers: Vec<String>,
}

impl ModuleSourceRegistrationFacts {
    pub(super) fn from_registration(registration: &Registration) -> Self {
        let kind = if registration.is_catalog() {
            ModuleSourceBucketKind::Catalog
        } else {
            ModuleSourceBucketKind::LocalFolder
        };
        let signer = registration.catalog_signer();
        Self {
            kind,
            trust_record_signer: registration
                .module_trust
                .as_ref()
                .map(|trust| trust.signer_key_id.clone()),
            pinned_signer: signer.map(|signer| signer.key_id.clone()),
            previous_signers: signer
                .map(|signer| signer.previous_signers.clone())
                .unwrap_or_default(),
            revoked_signers: signer
                .map(|signer| signer.revoked_signers.clone())
                .unwrap_or_default(),
        }
    }
}

/// Why one module release source is or is not trusted.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ModuleSourceTrustReason {
    /// The official `main` bucket is always a trusted module source.
    MainBucket,
    /// The release names no bucket and comes from a local registration.
    LocalRegistration,
    /// The release signer is authorized by the bucket's trusted signer.
    AuthorizedReleaseSigner {
        /// Registered bucket name.
        bucket: String,
        /// Signer recorded on the release.
        signer: String,
    },
    /// The release names a bucket that is not registered.
    UnregisteredBucket {
        /// Bucket named by the release source.
        bucket: String,
    },
    /// The bucket is a local folder bucket.
    LocalFolderBucket {
        /// Registered bucket name.
        bucket: String,
    },
    /// The bucket pins no catalog signer.
    NoPinnedSigner {
        /// Registered bucket name.
        bucket: String,
    },
    /// The bucket has no module trust record.
    NoTrustRecord {
        /// Registered bucket name.
        bucket: String,
    },
    /// The trust record names a signer that is not the pinned signer.
    SignerKeyChanged {
        /// Registered bucket name.
        bucket: String,
        /// Signer named by the module trust record.
        trust_record_signer: String,
        /// Signer the catalog currently pins.
        pinned_signer: String,
    },
    /// The release records no signer.
    MissingReleaseSigner {
        /// Registered bucket name.
        bucket: String,
    },
    /// The offline revocation key revoked the release signer.
    RevokedReleaseSigner {
        /// Registered bucket name.
        bucket: String,
        /// Signer recorded on the release.
        signer: String,
    },
    /// The release signer is neither the pinned signer nor a previous signer.
    UnauthorizedReleaseSigner {
        /// Registered bucket name.
        bucket: String,
        /// Signer recorded on the release.
        signer: String,
        /// Signer the catalog currently pins.
        pinned_signer: String,
    },
}

impl fmt::Display for ModuleSourceTrustReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MainBucket => {
                formatter.write_str("the official main bucket is always a trusted module source")
            }
            Self::LocalRegistration => {
                formatter.write_str("the release has no bucket and is a local registration")
            }
            Self::AuthorizedReleaseSigner { bucket, signer } => write!(
                formatter,
                "the release signer {signer} is authorized by the trusted signer of bucket {bucket}"
            ),
            Self::UnregisteredBucket { bucket } => {
                write!(formatter, "bucket {bucket} is not registered")
            }
            Self::LocalFolderBucket { bucket } => {
                formatter.write_str(&local_bucket_module_refusal(bucket))
            }
            Self::NoPinnedSigner { bucket } => {
                write!(formatter, "bucket {bucket} pins no catalog signer")
            }
            Self::NoTrustRecord { bucket } => {
                write!(formatter, "bucket {bucket} has no module trust record")
            }
            Self::SignerKeyChanged {
                bucket,
                trust_record_signer,
                pinned_signer,
            } => write!(
                formatter,
                "bucket {bucket} was trusted for signer {trust_record_signer}, but its pinned signer is {pinned_signer}"
            ),
            Self::MissingReleaseSigner { bucket } => {
                write!(formatter, "the release from bucket {bucket} records no signer")
            }
            Self::RevokedReleaseSigner { bucket, signer } => {
                write!(
                    formatter,
                    "the release signer {signer} was revoked by bucket {bucket}"
                )
            }
            Self::UnauthorizedReleaseSigner {
                bucket,
                signer,
                pinned_signer,
            } => write!(
                formatter,
                "the release signer {signer} is not authorized by the trusted signer {pinned_signer} of bucket {bucket}"
            ),
        }
    }
}

/// Decision for whether one module release source may supply executable code.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ModuleSourceTrustDecision {
    trusted: bool,
    reason: ModuleSourceTrustReason,
    remedy: Option<ModuleSourceTrustRemedy>,
}

impl ModuleSourceTrustDecision {
    fn trusted(reason: ModuleSourceTrustReason) -> Self {
        Self {
            trusted: true,
            reason,
            remedy: None,
        }
    }

    fn refused(reason: ModuleSourceTrustReason, bucket: &str) -> Self {
        Self {
            trusted: false,
            reason,
            remedy: Some(ModuleSourceTrustRemedy {
                bucket: bucket.to_owned(),
            }),
        }
    }

    /// True when the release source may supply executable game modules.
    #[must_use]
    pub fn is_trusted(&self) -> bool {
        self.trusted
    }

    /// The fact that decided this outcome.
    #[must_use]
    pub fn reason(&self) -> &ModuleSourceTrustReason {
        &self.reason
    }

    /// The user commands that can restore trust or remove the module.
    #[must_use]
    pub fn remedy(&self) -> Option<&ModuleSourceTrustRemedy> {
        self.remedy.as_ref()
    }
}

/// Remedy for a refused module source.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ModuleSourceTrustRemedy {
    bucket: String,
}

impl ModuleSourceTrustRemedy {
    /// Registered bucket name.
    #[must_use]
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// Command that trusts the bucket's current pinned signer for modules.
    #[must_use]
    pub fn trust_modules_command(&self) -> String {
        format!("modn bucket trust-modules {}", self.bucket)
    }

    /// Command that removes one game module instead of trusting its bucket.
    #[must_use]
    pub fn remove_module_command(&self, game: &str) -> String {
        format!("modn module remove {game}")
    }
}

/// Decide whether one module release source may supply executable game modules.
///
/// Owner rule, 2026-11-11: a release is trusted only when its recorded signer is
/// authorized by the signer that the bucket's trust record currently names.
/// `main` is always trusted, a release without a bucket comes from a local
/// registration, and a signer key change ends trust. A local folder bucket can
/// never supply modules. [`TrustedModuleSource::authorizes`] keeps a release
/// signed by a previous signer usable after a rotation, and refuses a release
/// signed by a revoked key.
///
/// `registration` holds the facts of the bucket named by `source`; `None` means
/// no such registration was found. The bucket name comes from `source`.
#[must_use]
pub fn decide_module_source_trust(
    source: &ModuleSource,
    registration: Option<&ModuleSourceRegistrationFacts>,
) -> ModuleSourceTrustDecision {
    let Some(bucket) = source.bucket.as_deref() else {
        return ModuleSourceTrustDecision::trusted(ModuleSourceTrustReason::LocalRegistration);
    };
    if bucket == "main" {
        return ModuleSourceTrustDecision::trusted(ModuleSourceTrustReason::MainBucket);
    }
    let Some(registration) = registration else {
        return ModuleSourceTrustDecision::refused(
            ModuleSourceTrustReason::UnregisteredBucket {
                bucket: bucket.to_owned(),
            },
            bucket,
        );
    };
    match registration.kind {
        ModuleSourceBucketKind::LocalFolder => ModuleSourceTrustDecision::refused(
            ModuleSourceTrustReason::LocalFolderBucket {
                bucket: bucket.to_owned(),
            },
            bucket,
        ),
        ModuleSourceBucketKind::Catalog => decide_catalog_trust(source, registration, bucket),
    }
}

fn decide_catalog_trust(
    source: &ModuleSource,
    registration: &ModuleSourceRegistrationFacts,
    bucket: &str,
) -> ModuleSourceTrustDecision {
    let Some(pinned_signer) = registration.pinned_signer.as_deref() else {
        return ModuleSourceTrustDecision::refused(
            ModuleSourceTrustReason::NoPinnedSigner {
                bucket: bucket.to_owned(),
            },
            bucket,
        );
    };
    let Some(trust_record_signer) = registration.trust_record_signer.as_deref() else {
        return ModuleSourceTrustDecision::refused(
            ModuleSourceTrustReason::NoTrustRecord {
                bucket: bucket.to_owned(),
            },
            bucket,
        );
    };
    if trust_record_signer != pinned_signer {
        return ModuleSourceTrustDecision::refused(
            ModuleSourceTrustReason::SignerKeyChanged {
                bucket: bucket.to_owned(),
                trust_record_signer: trust_record_signer.to_owned(),
                pinned_signer: pinned_signer.to_owned(),
            },
            bucket,
        );
    }
    let Some(release_signer) = source.signer_key_id.as_deref() else {
        return ModuleSourceTrustDecision::refused(
            ModuleSourceTrustReason::MissingReleaseSigner {
                bucket: bucket.to_owned(),
            },
            bucket,
        );
    };
    let trusted_source = TrustedModuleSource {
        bucket: bucket.to_owned(),
        signer_key_id: pinned_signer.to_owned(),
        previous_signers: registration.previous_signers.clone(),
        revoked_signers: registration.revoked_signers.clone(),
    };
    if trusted_source.authorizes(release_signer) {
        return ModuleSourceTrustDecision::trusted(
            ModuleSourceTrustReason::AuthorizedReleaseSigner {
                bucket: bucket.to_owned(),
                signer: release_signer.to_owned(),
            },
        );
    }
    let revoked = registration
        .revoked_signers
        .iter()
        .any(|signer| signer.eq_ignore_ascii_case(release_signer));
    if revoked {
        ModuleSourceTrustDecision::refused(
            ModuleSourceTrustReason::RevokedReleaseSigner {
                bucket: bucket.to_owned(),
                signer: release_signer.to_owned(),
            },
            bucket,
        )
    } else {
        ModuleSourceTrustDecision::refused(
            ModuleSourceTrustReason::UnauthorizedReleaseSigner {
                bucket: bucket.to_owned(),
                signer: release_signer.to_owned(),
                pinned_signer: pinned_signer.to_owned(),
            },
            bucket,
        )
    }
}

impl BucketStore {
    /// Decide whether one retained module release may supply executable code.
    ///
    /// Reads the registration of the bucket that `source` names and applies
    /// [`decide_module_source_trust`]. This method decides nothing itself.
    pub fn module_source_trust(&self, source: &ModuleSource) -> Result<ModuleSourceTrustDecision> {
        let facts = match source.bucket.as_deref() {
            None | Some("main") => None,
            Some(bucket) => self
                .registrations()?
                .get(bucket)
                .map(ModuleSourceRegistrationFacts::from_registration),
        };
        Ok(decide_module_source_trust(source, facts.as_ref()))
    }
}
