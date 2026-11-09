use crate::config::Config;
use crate::tools;
use serde_json::{json, Value};
use std::ffi::OsString;

pub const VERSION: &str = "2025-06-18";
pub const MESSAGE_LIMIT: usize = 64 * 1024;

#[derive(Default)]
pub struct Session {
    initialized: bool,
    ready: bool,
}

/// What the stdio loop must do after one input line.
pub enum Action {
    /// Write this response.
    Reply(Value),
    /// Write nothing.
    Silent,
    /// Run the CLI, which may take long enough to report progress.
    Call {
        id: Value,
        args: Vec<OsString>,
        progress_token: Option<Value>,
    },
}

pub fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

/// Whether `bytes` cancels the active request `id`.
pub fn cancels(bytes: &[u8], id: &Value) -> bool {
    let Ok(request) = serde_json::from_slice::<Value>(bytes) else {
        return false;
    };
    request["jsonrpc"] == "2.0"
        && request["method"] == "notifications/cancelled"
        && request.get("id").is_none()
        && request
            .get("params")
            .and_then(|params| params.get("requestId"))
            == Some(id)
}

impl Session {
    pub async fn handle(&mut self, config: &Config, bytes: &[u8]) -> Action {
        let request: Value = match serde_json::from_slice(bytes) {
            Ok(value) => value,
            Err(_) => return Action::Reply(error(Value::Null, -32700, "Invalid JSON")),
        };
        let id = request.get("id").cloned();
        let valid_id = id
            .as_ref()
            .is_none_or(|id| id.is_string() || id.as_i64().is_some() || id.as_u64().is_some());
        let Some(method) = request.get("method").and_then(Value::as_str) else {
            return Action::Reply(error(Value::Null, -32600, "Invalid request"));
        };
        if !request.is_object() || request["jsonrpc"] != "2.0" || !valid_id {
            return Action::Reply(error(Value::Null, -32600, "Invalid request"));
        }
        if request
            .get("params")
            .is_some_and(|params| !params.is_object())
        {
            return id.map_or(Action::Silent, |id| {
                Action::Reply(error(id, -32602, "Parameters must be an object"))
            });
        }
        let Some(id) = id else {
            if method == "notifications/initialized" && self.initialized {
                self.ready = true;
            }
            return Action::Silent;
        };
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
        let result = match method {
            "initialize" => {
                if self.initialized {
                    return Action::Reply(error(id, -32600, "Already initialized"));
                }
                if !params["protocolVersion"].is_string()
                    || !params["capabilities"].is_object()
                    || !params["clientInfo"]["name"].is_string()
                    || !params["clientInfo"]["version"].is_string()
                {
                    return Action::Reply(error(id, -32602, "Invalid initialize parameters"));
                }
                self.initialized = true;
                json!({"protocolVersion":VERSION,"capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"modn-mcp","version":env!("CARGO_PKG_VERSION")},"instructions":"Local scoped Modn CLI tools. Writes need server opt-in and explicit request approval. Previews never download. Progress uses _meta.progressToken; notifications/cancelled stops a call before its commit point."})
            }
            "ping" => json!({}),
            _ if !self.ready => {
                return Action::Reply(error(id, -32600, "Initialization must complete first"))
            }
            "tools/list" => {
                if !params.as_object().is_some_and(|object| object.is_empty()) {
                    return Action::Reply(error(id, -32602, "No pagination cursor is supported"));
                }
                tools::list(config.allow_writes)
            }
            "tools/call" => {
                let Some(name) = params["name"].as_str() else {
                    return Action::Reply(error(id, -32602, "Tool name is required"));
                };
                if !tools::known(name, config.allow_writes) {
                    return Action::Reply(error(id, -32602, "Unknown or disabled tool"));
                }
                if params.as_object().is_none_or(|object| {
                    object
                        .keys()
                        .any(|key| !matches!(key.as_str(), "name" | "arguments" | "_meta"))
                }) {
                    return Action::Reply(error(id, -32602, "Unknown call parameter"));
                }
                let progress_token = match params.get("_meta") {
                    None => None,
                    Some(meta) if !meta.is_object() => {
                        return Action::Reply(error(id, -32602, "_meta must be an object"))
                    }
                    Some(meta) => match meta.get("progressToken") {
                        None => None,
                        Some(token)
                            if token.is_string()
                                || token.as_i64().is_some()
                                || token.as_u64().is_some() =>
                        {
                            Some(token.clone())
                        }
                        Some(_) => {
                            return Action::Reply(error(id, -32602, "Invalid progress token"))
                        }
                    },
                };
                let input = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let args = match tools::arguments(config, name, input) {
                    Ok(args) => args,
                    Err(_) => {
                        return Action::Reply(error(
                            id,
                            -32602,
                            "Invalid tool arguments or disallowed path",
                        ))
                    }
                };
                return Action::Call {
                    id,
                    args,
                    progress_token,
                };
            }
            _ => return Action::Reply(error(id, -32601, "Method not found")),
        };
        Action::Reply(json!({"jsonrpc":"2.0","id":id,"result":result}))
    }
}
