//! Stable, machine-readable error codes for user-visible errors.
//!
//! Every error that a front end shows carries one [`ErrorCode`]. A specific
//! code names a condition a caller can act on. When no specific code applies,
//! the front end reports the default code of the command that failed, such as
//! [`ErrorCode::InstallFailed`]. Codes are stable: a released code keeps its
//! spelling and meaning.

use crate::progress::Cancelled;
use crate::transactions::NativeInjectorNotAllowed;
use serde::{Serialize, Serializer};
use std::fmt;

/// A stable, SCREAMING_SNAKE_CASE error code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorCode {
    /// The update target already uses the selected archive.
    AlreadyInstalled,
    /// A mod name matches several catalog mods.
    AmbiguousTarget,
    /// A write needs `--yes` or an interactive confirmation.
    ApprovalRequired,
    /// An archive cannot be read or changed during the operation.
    ArchiveInvalid,
    /// A mod has no downloadable source and needs `--file`.
    ArchiveRequired,
    /// The operation was cancelled before its commit point.
    Cancelled,
    /// A declared or ownership conflict blocks the operation.
    Conflict,
    /// A dependency is not installed or not intact.
    DependencyMissing,
    /// A downloaded or supplied archive does not match its pinned digest.
    DigestMismatch,
    /// A download did not complete.
    DownloadFailed,
    /// A game module failed to load or run.
    GameModuleError,
    /// Modn cannot select one registered game installation.
    GameNotSelected,
    /// A Git source resolved to another commit.
    GitSourceCommitMismatch,
    /// A Git source could not be fetched or read.
    GitSourceFetchFailed,
    /// A Git source contains an entry Modn does not install.
    GitSourceUnsupportedEntry,
    /// The command line is invalid.
    InvalidArguments,
    /// A filesystem operation failed.
    IoError,
    /// No catalog mod matches the target.
    ModNotFound,
    /// A native-injector package needs `--allow-native-injector`.
    NativeInjectorNotAllowed,
    /// Another installed package needs the selected package.
    PackageNeeded,
    /// No managed package matches the selector.
    PackageNotFound,
    /// The managed package is pinned.
    PackagePinned,
    /// An interrupted transaction must be recovered first.
    PendingTransaction,
    /// The installation plan has blockers.
    PlanBlocked,
    /// Manager state cannot be read or written.
    StateError,
    /// Default code of `modn adopt`.
    AdoptFailed,
    /// Default code of `modn asset`.
    AssetFailed,
    /// Default code of `modn auth`.
    AuthFailed,
    /// Default code of `modn bucket`.
    BucketFailed,
    /// Default code of `modn cache`.
    CacheFailed,
    /// Default code of `modn doctor`.
    DoctorFailed,
    /// Default code of `modn game`.
    GameFailed,
    /// Default code of `modn inspect`.
    InspectFailed,
    /// Default code of `modn install`.
    InstallFailed,
    /// Default code of `modn list`.
    ListFailed,
    /// Default code of `modn module`.
    ModuleFailed,
    /// Default code of `modn pin` and `modn unpin`.
    PinFailed,
    /// Default code of `modn recover`.
    RecoverFailed,
    /// Default code of `modn scan`.
    ScanFailed,
    /// Default code of `modn search`.
    SearchFailed,
    /// Default code of `modn uninstall`.
    UninstallFailed,
    /// Default code of `modn update`.
    UpdateFailed,
}

impl ErrorCode {
    /// Every code, in documentation order.
    pub const ALL: &'static [Self] = &[
        Self::AlreadyInstalled,
        Self::AmbiguousTarget,
        Self::ApprovalRequired,
        Self::ArchiveInvalid,
        Self::ArchiveRequired,
        Self::Cancelled,
        Self::Conflict,
        Self::DependencyMissing,
        Self::DigestMismatch,
        Self::DownloadFailed,
        Self::GameModuleError,
        Self::GameNotSelected,
        Self::GitSourceCommitMismatch,
        Self::GitSourceFetchFailed,
        Self::GitSourceUnsupportedEntry,
        Self::InvalidArguments,
        Self::IoError,
        Self::ModNotFound,
        Self::NativeInjectorNotAllowed,
        Self::PackageNeeded,
        Self::PackageNotFound,
        Self::PackagePinned,
        Self::PendingTransaction,
        Self::PlanBlocked,
        Self::StateError,
        Self::AdoptFailed,
        Self::AssetFailed,
        Self::AuthFailed,
        Self::BucketFailed,
        Self::CacheFailed,
        Self::DoctorFailed,
        Self::GameFailed,
        Self::InspectFailed,
        Self::InstallFailed,
        Self::ListFailed,
        Self::ModuleFailed,
        Self::PinFailed,
        Self::RecoverFailed,
        Self::ScanFailed,
        Self::SearchFailed,
        Self::UninstallFailed,
        Self::UpdateFailed,
    ];

    /// The code as it appears in JSON output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AlreadyInstalled => "ALREADY_INSTALLED",
            Self::AmbiguousTarget => "AMBIGUOUS_TARGET",
            Self::ApprovalRequired => "APPROVAL_REQUIRED",
            Self::ArchiveInvalid => "ARCHIVE_INVALID",
            Self::ArchiveRequired => "ARCHIVE_REQUIRED",
            Self::Cancelled => "CANCELLED",
            Self::Conflict => "CONFLICT",
            Self::DependencyMissing => "DEPENDENCY_MISSING",
            Self::DigestMismatch => "DIGEST_MISMATCH",
            Self::DownloadFailed => "DOWNLOAD_FAILED",
            Self::GameModuleError => "GAME_MODULE_ERROR",
            Self::GameNotSelected => "GAME_NOT_SELECTED",
            Self::GitSourceCommitMismatch => "GIT_SOURCE_COMMIT_MISMATCH",
            Self::GitSourceFetchFailed => "GIT_SOURCE_FETCH_FAILED",
            Self::GitSourceUnsupportedEntry => "GIT_SOURCE_UNSUPPORTED_ENTRY",
            Self::InvalidArguments => "INVALID_ARGUMENTS",
            Self::IoError => "IO_ERROR",
            Self::ModNotFound => "MOD_NOT_FOUND",
            Self::NativeInjectorNotAllowed => "NATIVE_INJECTOR_NOT_ALLOWED",
            Self::PackageNeeded => "PACKAGE_NEEDED",
            Self::PackageNotFound => "PACKAGE_NOT_FOUND",
            Self::PackagePinned => "PACKAGE_PINNED",
            Self::PendingTransaction => "PENDING_TRANSACTION",
            Self::PlanBlocked => "PLAN_BLOCKED",
            Self::StateError => "STATE_ERROR",
            Self::AdoptFailed => "ADOPT_FAILED",
            Self::AssetFailed => "ASSET_FAILED",
            Self::AuthFailed => "AUTH_FAILED",
            Self::BucketFailed => "BUCKET_FAILED",
            Self::CacheFailed => "CACHE_FAILED",
            Self::DoctorFailed => "DOCTOR_FAILED",
            Self::GameFailed => "GAME_FAILED",
            Self::InspectFailed => "INSPECT_FAILED",
            Self::InstallFailed => "INSTALL_FAILED",
            Self::ListFailed => "LIST_FAILED",
            Self::ModuleFailed => "MODULE_FAILED",
            Self::PinFailed => "PIN_FAILED",
            Self::RecoverFailed => "RECOVER_FAILED",
            Self::ScanFailed => "SCAN_FAILED",
            Self::SearchFailed => "SEARCH_FAILED",
            Self::UninstallFailed => "UNINSTALL_FAILED",
            Self::UpdateFailed => "UPDATE_FAILED",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for ErrorCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// An error with a stable code. Its message is the human-readable text.
#[derive(Debug)]
pub struct CodedError {
    code: ErrorCode,
    message: String,
}

impl CodedError {
    /// Creates an error with `code` and `message`.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl fmt::Display) -> Self {
        Self {
            code,
            message: message.to_string(),
        }
    }

    /// The stable code of this error.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        self.code
    }
}

impl fmt::Display for CodedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CodedError {}

/// Creates an [`anyhow::Error`] with `code` and `message`.
#[must_use]
pub fn coded(code: ErrorCode, message: impl fmt::Display) -> anyhow::Error {
    anyhow::Error::new(CodedError::new(code, message))
}

/// Gives `error` the code `code` unless it already has a specific code.
///
/// The message stays the same, including its context chain.
#[must_use]
pub fn with_code(error: anyhow::Error, code: ErrorCode) -> anyhow::Error {
    if code_of(&error).is_some() {
        return error;
    }
    coded(code, format!("{error:#}"))
}

/// Adds a stable code to the error of a fallible result.
pub trait ResultExt<T> {
    /// Gives the error the code `code` unless it already has a specific code.
    ///
    /// # Errors
    ///
    /// Returns the original error, with `code` when it had no specific code.
    fn code(self, code: ErrorCode) -> anyhow::Result<T>;
}

impl<T> ResultExt<T> for anyhow::Result<T> {
    fn code(self, code: ErrorCode) -> anyhow::Result<T> {
        self.map_err(|error| with_code(error, code))
    }
}

/// The specific code of `error`, or `None` when the error has none.
///
/// A cancellation, also one that a [`crate::progress::DownloadReader`] wraps
/// in an I/O error, wins over every other code. Otherwise the outermost
/// [`CodedError`] wins, then the code of a known error type in the chain, then
/// a legacy `CODE: message` prefix whose code is in [`ErrorCode::ALL`].
#[must_use]
pub fn code_of(error: &anyhow::Error) -> Option<ErrorCode> {
    let cancelled_read = error
        .downcast_ref::<std::io::Error>()
        .and_then(std::io::Error::get_ref)
        .is_some_and(|inner| inner.is::<Cancelled>());
    if cancelled_read || error.downcast_ref::<Cancelled>().is_some() {
        return Some(ErrorCode::Cancelled);
    }
    if error.downcast_ref::<NativeInjectorNotAllowed>().is_some() {
        return Some(ErrorCode::NativeInjectorNotAllowed);
    }
    if let Some(coded) = error.downcast_ref::<CodedError>() {
        return Some(coded.code);
    }
    if let Some(code) = error
        .chain()
        .filter_map(|cause| cause.to_string().split(':').next().map(str::to_owned))
        .find_map(|prefix| {
            ErrorCode::ALL
                .iter()
                .copied()
                .find(|code| code.as_str() == prefix)
        })
    {
        return Some(code);
    }
    if error.downcast_ref::<rusqlite::Error>().is_some() {
        return Some(ErrorCode::StateError);
    }
    if error.downcast_ref::<zip::result::ZipError>().is_some() {
        return Some(ErrorCode::ArchiveInvalid);
    }
    if error.downcast_ref::<std::io::Error>().is_some() {
        return Some(ErrorCode::IoError);
    }
    None
}

#[cfg(test)]
mod tests;
