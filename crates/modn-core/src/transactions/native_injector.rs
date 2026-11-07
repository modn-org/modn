//! Explicit policy for packages that inject native code into a game process.

use super::CatalogReference;
use anyhow::Result;
use std::fmt;

/// The flag a CLI caller passes to authorize a native injector.
pub const NATIVE_INJECTOR_FLAG: &str = "--allow-native-injector";

/// Stable machine-readable refusal code.
pub const NATIVE_INJECTOR_NOT_ALLOWED: &str = "NATIVE_INJECTOR_NOT_ALLOWED";

/// Whether the caller authorized managing a package marked as a native injector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeInjectorPolicy {
    /// Refuse any plan that contains a native injector.
    Refuse,
    /// Allow native injectors, after the caller's explicit opt-in.
    Allow,
}

/// Refusal returned when a native injector is planned without opt-in.
#[derive(Debug)]
pub struct NativeInjectorNotAllowed {
    package: String,
}

impl NativeInjectorNotAllowed {
    /// Refusal for the given catalog release.
    #[must_use]
    pub fn new(reference: &CatalogReference) -> Self {
        Self {
            package: reference.to_string(),
        }
    }

    /// Stable machine-readable error code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        NATIVE_INJECTOR_NOT_ALLOWED
    }

    /// Structured error envelope for `--json` output.
    #[must_use]
    pub fn as_json(&self) -> serde_json::Value {
        serde_json::json!({
            "error": self.code(),
            "package": self.package,
            "flag": NATIVE_INJECTOR_FLAG,
        })
    }
}

impl fmt::Display for NativeInjectorNotAllowed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} is a native injector; pass {NATIVE_INJECTOR_FLAG} to install or update it",
            self.package
        )
    }
}

impl std::error::Error for NativeInjectorNotAllowed {}

impl NativeInjectorPolicy {
    /// Refuse a package marked as a native injector unless the caller opted in.
    pub fn check(self, reference: &CatalogReference, native_injector: bool) -> Result<()> {
        if self == Self::Refuse && native_injector {
            return Err(NativeInjectorNotAllowed::new(reference).into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference() -> CatalogReference {
        CatalogReference::new("main".into(), "test-game".into(), "native-injector".into())
    }

    #[test]
    fn refuse_reports_the_package_code_and_flag() {
        let error = NativeInjectorPolicy::Refuse
            .check(&reference(), true)
            .expect_err("refused");
        let refusal = error
            .downcast_ref::<NativeInjectorNotAllowed>()
            .expect("typed refusal");
        assert_eq!(refusal.code(), "NATIVE_INJECTOR_NOT_ALLOWED");
        assert_eq!(
            refusal.as_json(),
            serde_json::json!({
                "error": "NATIVE_INJECTOR_NOT_ALLOWED",
                "package": "main/test-game/native-injector",
                "flag": "--allow-native-injector",
            })
        );
        assert!(format!("{refusal}").contains("--allow-native-injector"));
    }

    #[test]
    fn refuse_allows_ordinary_packages_and_allow_permits_injectors() {
        NativeInjectorPolicy::Refuse
            .check(&reference(), false)
            .expect("ordinary package");
        NativeInjectorPolicy::Allow
            .check(&reference(), true)
            .expect("explicit approval");
    }
}
