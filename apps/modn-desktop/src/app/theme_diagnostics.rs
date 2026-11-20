//! Sanitised theme diagnostics that the webview may render.
//!
//! A `THEME_INVALID` refusal carries a `diagnostics` array. Only the fields the
//! Theme control needs cross the webview boundary, and every string is cleaned
//! and bounded first. Raw envelope text never reaches the webview.

use serde::Serialize;
use serde_json::Value;

/// The most diagnostics one refusal may contribute to the Theme control.
const MAX_DIAGNOSTICS: usize = 50;
/// The longest rule id the control accepts.
const MAX_RULE_CHARS: usize = 64;
/// The longest message the control shows.
const MAX_MESSAGE_CHARS: usize = 300;
/// The longest theme label the control shows.
const MAX_THEME_CHARS: usize = 130;
/// The longest relative path the control shows.
const MAX_PATH_CHARS: usize = 200;

/// One validated theme problem shown by the Theme control.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ThemeDiagnostic {
    /// The `<game>/<id>` theme, when the CLI named one.
    pub(crate) theme: Option<String>,
    /// The file relative to the theme directory, when the CLI named one.
    pub(crate) path: Option<String>,
    /// The one-based line number, when the CLI named one.
    pub(crate) line: Option<u32>,
    /// The stable rule id.
    pub(crate) rule: String,
    /// The problem in words.
    pub(crate) message: String,
}

/// Reads and sanitises the `diagnostics` array of one `THEME_INVALID` envelope.
///
/// At most [`MAX_DIAGNOSTICS`] entries are read. A non-array value yields no
/// diagnostics.
pub(crate) fn sanitize(items: &[Value]) -> Vec<ThemeDiagnostic> {
    items
        .iter()
        .take(MAX_DIAGNOSTICS)
        .filter_map(sanitize_entry)
        .collect()
}

/// Reads one diagnostic object, or drops it when a required field is malformed.
fn sanitize_entry(value: &Value) -> Option<ThemeDiagnostic> {
    let object = value.as_object()?;
    let rule = clean(object.get("rule")?.as_str()?);
    if !valid_rule(&rule) {
        return None;
    }
    let message = truncate(clean(object.get("message")?.as_str()?), MAX_MESSAGE_CHARS);
    let theme = object
        .get("theme")
        .and_then(Value::as_str)
        .map(|theme| truncate(clean(theme), MAX_THEME_CHARS));
    let path = object
        .get("path")
        .and_then(Value::as_str)
        .map(clean)
        .filter(|path| safe_path(path));
    let line = object
        .get("line")
        .and_then(Value::as_u64)
        .and_then(|line| u32::try_from(line).ok());
    Some(ThemeDiagnostic {
        theme,
        path,
        line,
        rule,
        message,
    })
}

/// A rule id is one to 64 lowercase ASCII letters, digits or hyphens.
fn valid_rule(rule: &str) -> bool {
    !rule.is_empty()
        && rule.chars().count() <= MAX_RULE_CHARS
        && rule
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// Removes ASCII control characters, which could break the rendered line.
fn clean(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_ascii_control())
        .collect()
}

/// Cuts `text` to at most `limit` characters.
fn truncate(text: String, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text;
    }
    text.chars().take(limit).collect()
}

/// Whether a path may be shown: relative, no parent segment and bounded.
fn safe_path(path: &str) -> bool {
    !is_absolute(path) && !path.contains("..") && path.chars().count() <= MAX_PATH_CHARS
}

/// Whether a path is rooted: a slash, a backslash or a drive prefix.
fn is_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with('/')
        || path.starts_with('\\')
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
}

#[cfg(test)]
mod tests {
    use super::{ThemeDiagnostic, sanitize};
    use serde_json::{Value, json};

    /// Sanitises a JSON array and returns its first entry.
    fn first(value: Value) -> Option<ThemeDiagnostic> {
        sanitize(value.as_array().expect("a JSON array"))
            .into_iter()
            .next()
    }

    #[test]
    fn a_valid_entry_is_kept_as_is() {
        let entry = first(json!([{
            "theme": "cyberpunk-2077/night-city",
            "path": "theme.css",
            "line": 12,
            "rule": "unknown-property",
            "message": "property \"transform\" is not allowed"
        }]));
        assert_eq!(
            entry,
            Some(ThemeDiagnostic {
                theme: Some("cyberpunk-2077/night-city".to_owned()),
                path: Some("theme.css".to_owned()),
                line: Some(12),
                rule: "unknown-property".to_owned(),
                message: "property \"transform\" is not allowed".to_owned(),
            })
        );
    }

    #[test]
    fn missing_optional_fields_become_null() {
        let entry = first(json!([{"rule": "css-syntax", "message": "unexpected token"}]))
            .expect("the required fields are present");
        assert_eq!(entry.theme, None);
        assert_eq!(entry.path, None);
        assert_eq!(entry.line, None);
        let serialized = serde_json::to_value(&entry).expect("serializable diagnostic");
        assert_eq!(serialized["theme"], Value::Null);
        assert_eq!(serialized["path"], Value::Null);
        assert_eq!(serialized["line"], Value::Null);
    }

    #[test]
    fn explicit_null_optional_fields_stay_null() {
        let entry = first(json!([{
            "theme": null,
            "path": null,
            "line": null,
            "rule": "css-syntax",
            "message": "problem"
        }]))
        .expect("the required fields are present");
        assert_eq!(entry.theme, None);
        assert_eq!(entry.path, None);
        assert_eq!(entry.line, None);
    }

    #[test]
    fn at_most_fifty_entries_are_read() {
        let entries: Vec<Value> = (0..60)
            .map(|index| json!({"rule": "css-syntax", "message": format!("problem {index}")}))
            .collect();
        let diagnostics = sanitize(&entries);
        assert_eq!(diagnostics.len(), 50);
        assert_eq!(diagnostics[49].message, "problem 49");
    }

    #[test]
    fn a_bad_rule_drops_the_entry() {
        let too_long = "a".repeat(65);
        for rule in [
            json!(""),
            json!("Remote-Url"),
            json!("remote_url"),
            json!("remote url"),
            json!(too_long),
            json!(7),
        ] {
            assert!(
                first(json!([{"rule": rule, "message": "problem"}])).is_none(),
                "a malformed rule must drop the entry"
            );
        }
    }

    #[test]
    fn a_missing_or_non_string_message_drops_the_entry() {
        for message in [Value::Null, json!(7), json!(["text"])] {
            assert!(
                first(json!([{"rule": "css-syntax", "message": message}])).is_none(),
                "a malformed message must drop the entry"
            );
        }
        assert!(first(json!([{"rule": "css-syntax"}])).is_none());
    }

    #[test]
    fn control_characters_are_removed_from_every_string() {
        let entry = first(json!([{
            "theme": "cyberpunk-2077/night\u{7}city",
            "path": "theme\u{0}.css",
            "rule": "css-syntax",
            "message": "line one\u{7}\nline two"
        }]))
        .expect("the entry stays valid");
        assert_eq!(entry.theme.as_deref(), Some("cyberpunk-2077/nightcity"));
        assert_eq!(entry.path.as_deref(), Some("theme.css"));
        assert_eq!(entry.message, "line oneline two");
    }

    #[test]
    fn message_and_theme_are_truncated() {
        let message = "m".repeat(400);
        let theme = "t".repeat(200);
        let entry = first(json!([{
            "theme": theme,
            "rule": "css-syntax",
            "message": message
        }]))
        .expect("long text is still valid");
        assert_eq!(entry.message.chars().count(), 300);
        assert_eq!(entry.theme.expect("a theme").chars().count(), 130);
    }

    #[test]
    fn an_absolute_or_parent_path_becomes_null() {
        let long = "a".repeat(201);
        for path in [
            json!("/etc/theme.css"),
            json!("\\theme.css"),
            json!("C:\\themes\\theme.css"),
            json!("C:theme.css"),
            json!("assets/../theme.css"),
            json!(long),
        ] {
            let entry = first(json!([{"path": path, "rule": "css-syntax", "message": "problem"}]))
                .expect("the entry stays valid");
            assert_eq!(entry.path, None, "the unsafe path must be dropped");
        }
    }

    #[test]
    fn unknown_keys_are_dropped() {
        let entry = first(json!([{
            "theme": null,
            "path": "theme.css",
            "line": 3,
            "rule": "css-syntax",
            "message": "problem",
            "secret": "raw backend text"
        }]))
        .expect("the entry stays valid");
        let serialized = serde_json::to_value(&entry).expect("serializable diagnostic");
        assert_eq!(
            serialized,
            json!({
                "theme": null,
                "path": "theme.css",
                "line": 3,
                "rule": "css-syntax",
                "message": "problem"
            })
        );
    }

    #[test]
    fn non_object_entries_are_dropped() {
        let diagnostics = sanitize(
            json!([
                "raw text",
                7,
                null,
                ["rule"],
                {"rule": "css-syntax", "message": "kept"}
            ])
            .as_array()
            .expect("a JSON array"),
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].message, "kept");
    }

    #[test]
    fn a_line_that_is_not_a_u32_becomes_null() {
        for line in [json!(-1), json!(1.5), json!(4_294_967_296_u64), json!("12")] {
            let entry = first(json!([{"line": line, "rule": "css-syntax", "message": "problem"}]))
                .expect("the entry stays valid");
            assert_eq!(entry.line, None);
        }
    }

    /// A refusal envelope with one valid and one malformed diagnostic.
    const THEME_REFUSAL: &str = r#"{"error":"THEME_INVALID","message":"the theme is not valid","diagnostics":[{"theme":"cyberpunk-2077/night-city","path":"theme.css","line":12,"rule":"unknown-property","message":"property \"transform\" is not allowed"},{"rule":"Bad Rule","message":"dropped"}]}"#;

    #[test]
    fn a_progress_refusal_converts_theme_diagnostics_to_the_webview_shape() {
        let error = crate::backend::BackendError::refused_from_envelope_for_test(THEME_REFUSAL);
        let mapped = crate::app::error::CommandError::from(error);
        assert_eq!(mapped.code, "THEME_INVALID");
        assert_eq!(
            mapped.message,
            "The theme was refused. Its diagnostics name each problem."
        );
        let json = serde_json::to_value(&mapped).expect("serializable error");
        assert_eq!(json["code"], "THEME_INVALID");
        assert_eq!(
            json["diagnostics"],
            json!([{
                "theme": "cyberpunk-2077/night-city",
                "path": "theme.css",
                "line": 12,
                "rule": "unknown-property",
                "message": "property \"transform\" is not allowed"
            }])
        );
        assert!(!json.to_string().contains("Bad Rule"));
    }

    #[test]
    fn the_anyhow_path_converts_theme_diagnostics_to_the_same_shape() {
        let text = format!("Backend refused (exit 1): {THEME_REFUSAL}\n");
        let mapped = crate::app::error::CommandError::from(anyhow::anyhow!(text));
        let json = serde_json::to_value(&mapped).expect("serializable error");
        assert_eq!(json["code"], "THEME_INVALID");
        assert_eq!(
            json["diagnostics"],
            json!([{
                "theme": "cyberpunk-2077/night-city",
                "path": "theme.css",
                "line": 12,
                "rule": "unknown-property",
                "message": "property \"transform\" is not allowed"
            }])
        );
    }

    #[test]
    fn an_error_without_diagnostics_omits_the_key() {
        let plain =
            crate::app::error::CommandError::from(anyhow::anyhow!("opaque transport failure"));
        let json = serde_json::to_value(&plain).expect("serializable error");
        assert!(json.get("diagnostics").is_none());

        let other = r#"Backend refused (exit 1): {"error":"CONFLICT","diagnostics":[{"rule":"css-syntax","message":"secret"}]}"#;
        let mapped = crate::app::error::CommandError::from(anyhow::anyhow!(other));
        let json = serde_json::to_value(&mapped).expect("serializable error");
        assert!(json.get("diagnostics").is_none());
    }
}
