use crate::backend::{BackendError, BackendErrorKind};
use serde::Serialize;
use serde_json::Value;

/// The capabilities an approval refusal leaves uncovered, in the shape the
/// webview's ask flow sends back as command arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ApprovalRequest {
    #[serde(rename = "nativeInjector")]
    pub(crate) native_injector: bool,
    pub(crate) executables: Vec<String>,
}

/// The only error shape the webview receives. `code` is a CLI stable code, or
/// `BACKEND_FAILED` / `BACKEND_TIMEOUT` when the backend itself failed. Raw
/// backend output never crosses this boundary.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct CommandError {
    pub(crate) code: String,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) approval: Option<ApprovalRequest>,
}

impl CommandError {
    pub(crate) fn new(code: &str) -> Self {
        Self {
            code: code.to_string(),
            message: short_message(code).to_string(),
            approval: None,
        }
    }

    /// Refuses a write until the webview approves the named capabilities.
    pub(crate) fn approval_required(native_injector: bool, executables: Vec<String>) -> Self {
        let mut error = Self::new("APPROVAL_REQUIRED");
        error.approval = Some(ApprovalRequest {
            native_injector,
            executables,
        });
        error
    }

    /// Refuses a write whose archive changed after the review.
    pub(crate) fn digest_mismatch() -> Self {
        Self {
            code: "DIGEST_MISMATCH".to_string(),
            message: "The archive changed after review. Choose it and try again.".to_string(),
            approval: None,
        }
    }

    /// Refuses a write whose installation plan changed after the review.
    pub(crate) fn plan_changed() -> Self {
        Self {
            code: "PLAN_BLOCKED".to_string(),
            message: "The installation plan changed. Review the archive again.".to_string(),
            approval: None,
        }
    }

    /// Refuses two writes that reuse one run id.
    pub(crate) fn run_in_use() -> Self {
        Self {
            code: "RUN_ID_IN_USE".to_string(),
            message: "Another operation is already using this run.".to_string(),
            approval: None,
        }
    }

    pub(crate) fn backend_failed() -> Self {
        Self::new("BACKEND_FAILED")
    }

    /// Reports a cancel that arrived before the write started.
    pub(crate) fn cancelled() -> Self {
        Self::new("CANCELLED")
    }

    /// Rejects a package the inventory no longer lists.
    pub(crate) fn package_not_found() -> Self {
        Self::new("MOD_NOT_FOUND")
    }

    /// Rejects an update of a package with no catalog provenance.
    pub(crate) fn update_not_in_catalog() -> Self {
        Self::new("UPDATE_FAILED")
    }

    /// Rejects an executable path before any CLI process starts.
    pub(crate) fn invalid_executable() -> Self {
        Self {
            code: "INVALID_EXECUTABLE".to_string(),
            message:
                "Executable approval paths must be game-relative, non-empty, and must not start with '-'."
                    .to_string(),
            approval: None,
        }
    }
}

impl From<anyhow::Error> for CommandError {
    fn from(error: anyhow::Error) -> Self {
        let code = classify(&error);
        let mut mapped = Self::new(&code);
        mapped.approval = approval_from_error(&error);
        mapped
    }
}

impl From<BackendError> for CommandError {
    fn from(error: BackendError) -> Self {
        let mut mapped = if error.kind() == BackendErrorKind::Timeout {
            Self::new("BACKEND_TIMEOUT")
        } else {
            Self::new(error.code().unwrap_or("BACKEND_FAILED"))
        };
        if mapped.code == "NATIVE_INJECTOR_NOT_ALLOWED" {
            mapped.approval = Some(ApprovalRequest {
                native_injector: true,
                executables: Vec::new(),
            });
        }
        mapped
    }
}

pub(crate) fn plan_blocked() -> anyhow::Error {
    anyhow::anyhow!("PLAN_BLOCKED: Modn blocked this operation")
}

/// The approval a CLI refusal names, when the envelope carries one.
///
/// A native-injector refusal without an `uncovered` object still names the
/// capability: the flag it asks for is the whole approval.
fn approval_from_error(error: &anyhow::Error) -> Option<ApprovalRequest> {
    let text = format!("{error:#}");
    let object = first_error_envelope(&text)?;
    let code = object.get("error").and_then(Value::as_str)?;
    if let Some(uncovered) = object.get("uncovered").and_then(Value::as_object) {
        let native_injector = uncovered
            .get("native_injector")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let executables: Vec<String> = uncovered
            .get("executables")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if native_injector || !executables.is_empty() {
            return Some(ApprovalRequest {
                native_injector,
                executables,
            });
        }
    }
    if code == "NATIVE_INJECTOR_NOT_ALLOWED" {
        return Some(ApprovalRequest {
            native_injector: true,
            executables: Vec::new(),
        });
    }
    None
}

fn classify(error: &anyhow::Error) -> String {
    let text = format!("{error:#}");
    if text.contains("Backend exceeded 60 seconds") {
        return "BACKEND_TIMEOUT".to_string();
    }
    if text.starts_with("PLAN_BLOCKED:") {
        return "PLAN_BLOCKED".to_string();
    }
    error_code_from_json(&text).unwrap_or_else(|| "BACKEND_FAILED".to_string())
}

/// Reads the CLI error envelope that `Backend::call` embeds in its refusal
/// message. Only the stable code is kept; the message text is discarded.
fn error_code_from_json(text: &str) -> Option<String> {
    first_error_envelope(text).and_then(|object| {
        object
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_string)
    })
}

fn first_error_envelope(text: &str) -> Option<serde_json::Map<String, Value>> {
    let mut rest = text;
    while let Some(index) = rest.find('{') {
        let candidate = &rest[index..];
        let mut values = serde_json::Deserializer::from_str(candidate).into_iter::<Value>();
        if let Some(Ok(Value::Object(object))) = values.next()
            && let Some(code) = object.get("error").and_then(Value::as_str)
            && is_code(code)
        {
            return Some(object);
        }
        rest = &candidate[1..];
    }
    None
}

fn is_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 64
        && code
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

fn short_message(code: &str) -> &'static str {
    match code {
        "BACKEND_TIMEOUT" => "The operation took too long. Check diagnostics before trying again.",
        "BACKEND_FAILED" => "The operation could not be completed.",
        "MOD_NOT_FOUND" => "The catalog does not list that package.",
        "UPDATE_FAILED" => "This package cannot be updated from the catalog.",
        "PLAN_BLOCKED" => "Modn blocked this operation. Open details for the reason.",
        "CONFLICT" => "Another package owns these files.",
        "APPROVAL_REQUIRED" => "This change needs your approval.",
        "NATIVE_INJECTOR_NOT_ALLOWED" => "This mod runs native code and needs your approval.",
        "PENDING_TRANSACTION" => "An earlier operation needs recovery.",
        "STATE_ERROR" | "IO_ERROR" => "Modn could not read or write its files.",
        "CANCELLED" => "The operation was cancelled.",
        _ => "The operation could not be completed.",
    }
}

#[cfg(test)]
mod tests {
    use super::CommandError;

    #[test]
    fn cli_envelope_code_is_kept_without_cli_text() {
        let error = anyhow::anyhow!(
            "Backend refused (exit 1): {{\"error\":\"PLAN_BLOCKED\",\"message\":\"secret detail\"}}\n"
        );
        let mapped = CommandError::from(error);
        assert_eq!(mapped.code, "PLAN_BLOCKED");
        assert!(!mapped.message.contains("secret"));
    }

    #[test]
    fn timeout_and_unknown_failures_use_backend_codes() {
        let timeout = CommandError::from(anyhow::anyhow!(
            "Backend exceeded 60 seconds. A write may need recovery"
        ));
        assert_eq!(timeout.code, "BACKEND_TIMEOUT");
        let unknown = CommandError::from(anyhow::anyhow!("opaque transport failure"));
        assert_eq!(unknown.code, "BACKEND_FAILED");
    }

    #[test]
    fn plan_blocked_prefix_maps_to_the_stable_code() {
        let mapped = CommandError::from(super::plan_blocked());
        assert_eq!(mapped.code, "PLAN_BLOCKED");
    }

    #[test]
    fn an_approval_refusal_keeps_the_uncovered_executables() {
        let error = anyhow::anyhow!(
            "Backend refused (exit 1): {{\"error\":\"APPROVAL_REQUIRED\",\"uncovered\":{{\"native_injector\":false,\"executables\":[\"tools/helper.exe\"]}}}}\n"
        );
        let mapped = CommandError::from(error);
        assert_eq!(mapped.code, "APPROVAL_REQUIRED");
        let approval = mapped
            .approval
            .as_ref()
            .expect("an approval refusal names its capabilities");
        assert!(!approval.native_injector);
        assert_eq!(approval.executables, ["tools/helper.exe"]);
        let json = serde_json::to_value(&mapped).expect("serializable error");
        assert_eq!(json["approval"]["nativeInjector"], false);
        assert_eq!(json["approval"]["executables"][0], "tools/helper.exe");
    }

    #[test]
    fn a_native_injector_refusal_names_the_native_capability() {
        let error = anyhow::anyhow!(
            "Backend refused (exit 1): {{\"error\":\"NATIVE_INJECTOR_NOT_ALLOWED\",\"package\":\"main/game/mod\",\"flag\":\"--allow-native-injector\"}}"
        );
        let mapped = CommandError::from(error);
        assert_eq!(mapped.code, "NATIVE_INJECTOR_NOT_ALLOWED");
        let approval = mapped.approval.expect("the refusal names native code");
        assert!(approval.native_injector);
        assert!(approval.executables.is_empty());
    }

    #[test]
    fn other_failures_carry_no_approval() {
        let mapped = CommandError::from(anyhow::anyhow!(
            "Backend refused (exit 1): {{\"error\":\"CONFLICT\",\"message\":\"secret\"}}"
        ));
        assert_eq!(mapped.code, "CONFLICT");
        assert!(mapped.approval.is_none());
        let serialized = serde_json::to_value(&mapped).expect("serializable error");
        assert!(serialized.get("approval").is_none());
    }
}
