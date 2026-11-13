use crate::backend::{BackendError, BackendErrorKind};
use serde::Serialize;
use serde_json::Value;

/// The only error shape the webview receives. `code` is a CLI stable code, or
/// `BACKEND_FAILED` / `BACKEND_TIMEOUT` when the backend itself failed. Raw
/// backend output never crosses this boundary.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct CommandError {
    pub(crate) code: String,
    pub(crate) message: String,
}

impl CommandError {
    pub(crate) fn new(code: &str) -> Self {
        Self {
            code: code.to_string(),
            message: short_message(code).to_string(),
        }
    }

    pub(crate) fn backend_failed() -> Self {
        Self::new("BACKEND_FAILED")
    }

    /// Rejects an executable path before any CLI process starts.
    pub(crate) fn invalid_executable() -> Self {
        Self {
            code: "INVALID_EXECUTABLE".to_string(),
            message:
                "Executable approval paths must be game-relative, non-empty, and must not start with '-'."
                    .to_string(),
        }
    }
}

impl From<anyhow::Error> for CommandError {
    fn from(error: anyhow::Error) -> Self {
        let code = classify(&error);
        Self::new(&code)
    }
}

impl From<BackendError> for CommandError {
    fn from(error: BackendError) -> Self {
        if error.kind() == BackendErrorKind::Timeout {
            return Self::new("BACKEND_TIMEOUT");
        }
        Self::new(error.code().unwrap_or("BACKEND_FAILED"))
    }
}

pub(crate) fn plan_blocked() -> anyhow::Error {
    anyhow::anyhow!("PLAN_BLOCKED: Modn blocked this operation")
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
    let mut rest = text;
    while let Some(index) = rest.find('{') {
        let candidate = &rest[index..];
        let mut values = serde_json::Deserializer::from_str(candidate).into_iter::<Value>();
        if let Some(Ok(Value::Object(object))) = values.next()
            && let Some(code) = object.get("error").and_then(Value::as_str)
            && is_code(code)
        {
            return Some(code.to_string());
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
}
