//! The error type of [`Backend::call_with_progress`](super::Backend::call_with_progress).

use serde_json::Value;
use std::io;
use std::process::ExitStatus;
use std::time::Duration;

/// The capabilities an approval refusal leaves uncovered.
///
/// The CLI writes them in its stderr error envelope, for example
/// `{"error":"NATIVE_INJECTOR_NOT_ALLOWED","uncovered":{"native_injector":true,"executables":["tools/helper.exe"]}}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UncoveredApproval {
    /// Whether the refusal names native-injector code.
    pub(crate) native_injector: bool,
    /// The exact game-relative executable paths the refusal leaves uncovered.
    pub(crate) executables: Vec<String>,
    /// Whether the envelope named every uncovered capability.
    ///
    /// An envelope without an `executables` list is incomplete: callers read
    /// the installation plan instead of retrying with an empty list.
    pub(crate) complete: bool,
}

impl UncoveredApproval {
    /// Reads the `uncovered` object of one CLI error envelope.
    fn from_envelope(value: &Value) -> Option<Self> {
        let uncovered = value.get("uncovered")?.as_object()?;
        let executables = uncovered.get("executables");
        Some(Self {
            native_injector: uncovered
                .get("native_injector")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            executables: executables
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            complete: executables.is_some(),
        })
    }
}

/// One CLI error envelope from stderr.
///
/// The envelope carries the stable code, for an approval refusal the
/// capabilities that remain uncovered, and for `THEME_INVALID` the raw
/// diagnostic objects the app layer sanitises.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ErrorEnvelope {
    pub(super) code: String,
    pub(super) approval: Option<UncoveredApproval>,
    pub(super) diagnostics: Vec<Value>,
}

impl ErrorEnvelope {
    pub(super) fn parse(value: &Value) -> Option<Self> {
        let code = value.get("error")?.as_str()?.to_owned();
        // Any other code's diagnostics key is untrusted noise and is ignored.
        let diagnostics = if code == "THEME_INVALID" {
            value
                .get("diagnostics")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        Some(Self {
            code,
            approval: UncoveredApproval::from_envelope(value),
            diagnostics,
        })
    }
}

/// A failed progress-aware backend call.
///
/// The message never contains raw CLI stderr text. Callers render
/// [`BackendError::code`] through their own short-message map.
#[derive(Debug)]
pub(crate) struct BackendError {
    kind: BackendErrorKind,
    code: Option<String>,
    approval: Option<UncoveredApproval>,
    diagnostics: Vec<Value>,
    message: String,
}

impl BackendError {
    pub(super) fn cancelled() -> Self {
        Self {
            kind: BackendErrorKind::Cancelled,
            code: Some("CANCELLED".into()),
            approval: None,
            diagnostics: Vec::new(),
            message: "Backend cancelled before the commit point; nothing changed".into(),
        }
    }

    pub(super) fn timeout(timeout: Duration) -> Self {
        Self {
            kind: BackendErrorKind::Timeout,
            code: None,
            approval: None,
            diagnostics: Vec::new(),
            message: format!(
                "Backend made no progress for {:.1} seconds before its commit point; the operation was cancelled and stopped",
                timeout.as_secs_f64()
            ),
        }
    }

    pub(super) fn refused(
        exit: ExitStatus,
        code: Option<String>,
        approval: Option<UncoveredApproval>,
        diagnostics: Vec<Value>,
    ) -> Self {
        let message = match &code {
            Some(code) => format!("Backend refused ({exit}): {code}"),
            None => format!("Backend refused ({exit})"),
        };
        Self {
            kind: BackendErrorKind::Refused,
            code,
            approval,
            diagnostics,
            message,
        }
    }

    pub(super) fn output() -> Self {
        Self {
            kind: BackendErrorKind::Output,
            code: None,
            approval: None,
            diagnostics: Vec::new(),
            message: "Backend output exceeded its bound or could not be read".into(),
        }
    }

    /// The write committed, but its result could not be read or trusted.
    ///
    /// The call never kills a writer after the commit event, so the installed
    /// files stay in place. A lost stdout result and a lost progress stream
    /// both leave the view stale. Callers treat this code as
    /// success-with-stale-view and refresh rather than reporting a failed
    /// write.
    pub(super) fn output_after_commit() -> Self {
        Self {
            kind: BackendErrorKind::Committed,
            code: Some("BACKEND_OUTPUT_TRUNCATED_AFTER_COMMIT".into()),
            approval: None,
            diagnostics: Vec::new(),
            message: "Backend committed the operation but its output could not be read or exceeded the output bound; refresh to see the result".into(),
        }
    }

    pub(super) fn spawn(error: io::Error) -> Self {
        Self {
            kind: BackendErrorKind::Spawn,
            code: None,
            approval: None,
            diagnostics: Vec::new(),
            message: format!("Cannot start configured backend: {error}"),
        }
    }

    pub(super) fn io(message: &str) -> Self {
        Self {
            kind: BackendErrorKind::Output,
            code: None,
            approval: None,
            diagnostics: Vec::new(),
            message: message.into(),
        }
    }

    pub(super) fn invalid_json() -> Self {
        Self {
            kind: BackendErrorKind::InvalidJson,
            code: None,
            approval: None,
            diagnostics: Vec::new(),
            message: "Backend returned invalid JSON; verify the configured CLI version".into(),
        }
    }

    /// Why the call failed.
    #[must_use]
    pub(crate) fn kind(&self) -> BackendErrorKind {
        self.kind
    }

    /// The CLI's stable error code, when it reported one.
    ///
    /// A run cancelled before the commit point reports `CANCELLED` even when
    /// the CLI exits without its error envelope.
    #[must_use]
    pub(crate) fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }

    /// The capabilities a refusal left uncovered, when its envelope named any.
    #[must_use]
    pub(crate) fn approval(&self) -> Option<&UncoveredApproval> {
        self.approval.as_ref()
    }

    /// The raw `THEME_INVALID` diagnostics, which the app layer sanitises.
    #[must_use]
    pub(crate) fn diagnostics(&self) -> &[Value] {
        &self.diagnostics
    }
}

#[cfg(test)]
impl BackendError {
    /// Builds a refusal for tests that cannot spawn the CLI.
    pub(crate) fn refused_for_test(code: &str, approval: Option<UncoveredApproval>) -> Self {
        Self {
            kind: BackendErrorKind::Refused,
            code: Some(code.into()),
            approval,
            diagnostics: Vec::new(),
            message: String::new(),
        }
    }

    /// Builds the refusal one CLI envelope text describes, for tests.
    pub(crate) fn refused_from_envelope_for_test(text: &str) -> Self {
        let value: Value = serde_json::from_str(text).expect("a JSON envelope");
        let envelope = ErrorEnvelope::parse(&value).expect("a CLI error envelope");
        Self {
            kind: BackendErrorKind::Refused,
            code: Some(envelope.code),
            approval: envelope.approval,
            diagnostics: envelope.diagnostics,
            message: String::new(),
        }
    }
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for BackendError {}

/// Why a progress-aware backend call failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BackendErrorKind {
    /// The CLI stopped before the commit point because the caller cancelled.
    Cancelled,
    /// The CLI produced no output before the inactivity timeout, so it was stopped.
    Timeout,
    /// The CLI exited with a failure.
    Refused,
    /// A stream exceeded its bound or could not be read.
    Output,
    /// The write committed, but its result could not be read within the bound
    /// or a stream failed after the commit.
    ///
    /// The operation committed; callers refresh their view instead of
    /// reporting a failed write.
    Committed,
    /// The configured executable could not start.
    Spawn,
    /// The CLI exited successfully but stdout was not one JSON value.
    InvalidJson,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_envelope_keeps_the_native_flag_and_exact_executables() {
        let value = json!({
            "error": "NATIVE_INJECTOR_NOT_ALLOWED",
            "package": "main/fixture/mod",
            "uncovered": {
                "native_injector": true,
                "executables": ["tools/native-helper.exe", "engine/tools/inject.exe"]
            },
            "flags": ["--allow-native-injector", "--allow-executable"]
        });
        let envelope = ErrorEnvelope::parse(&value).expect("a CLI error envelope");
        assert_eq!(envelope.code, "NATIVE_INJECTOR_NOT_ALLOWED");
        let approval = envelope.approval.expect("uncovered capabilities");
        assert!(approval.native_injector);
        assert_eq!(
            approval.executables,
            ["tools/native-helper.exe", "engine/tools/inject.exe"]
        );
        assert!(approval.complete);
    }

    #[test]
    fn an_envelope_without_executables_is_incomplete() {
        let value = json!({
            "error": "NATIVE_INJECTOR_NOT_ALLOWED",
            "uncovered": {"native_injector": true}
        });
        let approval = ErrorEnvelope::parse(&value)
            .and_then(|envelope| envelope.approval)
            .expect("uncovered capabilities");
        assert!(approval.native_injector);
        assert!(approval.executables.is_empty());
        assert!(
            !approval.complete,
            "an empty list without the key must not assert that no executable is uncovered"
        );
    }

    #[test]
    fn a_theme_refusal_keeps_its_diagnostics() {
        let value = json!({
            "error": "THEME_INVALID",
            "message": "the theme is not valid",
            "diagnostics": [{
                "theme": "cyberpunk-2077/night-city",
                "path": "theme.css",
                "line": 12,
                "rule": "unknown-property",
                "message": "property \"transform\" is not allowed"
            }]
        });
        let envelope = ErrorEnvelope::parse(&value).expect("a CLI error envelope");
        assert_eq!(envelope.code, "THEME_INVALID");
        assert_eq!(envelope.diagnostics.len(), 1);
        assert_eq!(envelope.diagnostics[0]["rule"], "unknown-property");
    }

    #[test]
    fn a_diagnostics_key_is_ignored_for_other_codes() {
        let value = json!({
            "error": "PLAN_BLOCKED",
            "message": "the plan is blocked",
            "diagnostics": [{"rule": "css-syntax", "message": "unexpected token"}]
        });
        let envelope = ErrorEnvelope::parse(&value).expect("a CLI error envelope");
        assert_eq!(envelope.code, "PLAN_BLOCKED");
        assert!(envelope.diagnostics.is_empty());
    }
}
