mod dependencies;
mod git;
mod lock;
mod manifest;
mod signature;
mod snapshot;
mod store;
mod trust;
mod update;
mod version;

pub use crate::bucket_digest::CatalogDigest;
pub use crate::game_modules::{CatalogModuleReport, IgnoredModule, ModuleIgnoreReason};
pub use dependencies::install_order;
pub use git::{CatalogSigner, GitCatalogRevision, GitCatalogSource, GitCatalogSync};
pub use manifest::{ArchiveFormat, PackageManifest, PackageSource};
pub use signature::{
    catalog_statement, verify_catalog, verify_catalog_directory, CatalogKeyId, CatalogPublicKey,
    CatalogStatement, VerifiedCatalog,
};
pub use snapshot::BucketSnapshot;
pub use store::{BucketInfo, BucketPackage, BucketStore};
pub use trust::{
    ModuleSourceTrust, ModuleTrustAck, ModuleTrustState, TrustModulesOutcome, TrustedModuleSource,
};
pub use version::VersionReq;
