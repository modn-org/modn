mod checkver;
mod dependencies;
mod git;
mod lock;
mod manifest;
mod mirrors;
mod retire;
mod rotation;
mod signature;
mod snapshot;
mod store;
mod test_hooks;
mod trust;
mod trust_decision;
mod update;
mod version;
mod zip_source;

pub use crate::bucket_digest::CatalogDigest;
pub use crate::game_modules::{CatalogModuleReport, IgnoredModule, ModuleIgnoreReason};
pub use checkver::Checkver;
pub use dependencies::install_order;
pub use git::{CatalogSigner, GitCatalogRevision, GitCatalogSource, GitCatalogSync};
pub use manifest::{ArchiveFormat, PackageManifest, PackageSource};
pub use mirrors::{CatalogHost, CatalogSyncTrace, MAX_CATALOG_MIRRORS};
pub use rotation::{
    key_statement_bytes, revocation_statement, rotation_statement, CatalogKeyChange, KeyChangeKind,
};
pub use signature::{
    catalog_statement, verify_catalog, verify_catalog_directory, CatalogKeyId, CatalogPublicKey,
    CatalogStatement, VerifiedCatalog,
};
pub use snapshot::BucketSnapshot;
pub use store::{validate_bucket_metadata, BucketInfo, BucketPackage, BucketStore};
pub use test_hooks::main_release_list_override;
pub use trust::{
    local_bucket_module_refusal, ModuleSourceTrust, ModuleTrustAck, ModuleTrustState,
    TrustModulesOutcome, TrustedModuleSource,
};
pub use trust_decision::{
    decide_module_source_trust, ModuleSourceBucketKind, ModuleSourceRegistrationFacts,
    ModuleSourceTrustDecision, ModuleSourceTrustReason, ModuleSourceTrustRemedy,
};
pub use version::VersionReq;
pub use zip_source::{ZipCatalogRevision, ZipCatalogSource, ZipCatalogSync};
