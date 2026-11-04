use crate::config::Config;
use crate::tools;
use serde_json::{json, Value};

pub const VERSION: &str = "2025-06-18";
pub const MESSAGE_LIMIT: usize = 64 * 1024;

#[derive(Default)]
pub struct Session {
    initialized: bool,
    ready: bool,
}

pub fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

impl Session {
    pub async fn handle(&mut self, config: &Config, bytes: &[u8]) -> Option<Value> {
        let request: Value = match serde_json::from_slice(bytes) {
            Ok(value) => value,
            Err(_) => return Some(error(Value::Null, -32700, "Invalid JSON")),
        };
        let id = request.get("id").cloned();
        let valid_id = id
            .as_ref()
            .is_none_or(|id| id.is_string() || id.as_i64().is_some() || id.as_u64().is_some());
        let Some(method) = request.get("method").and_then(Value::as_str) else {
            return Some(error(Value::Null, -32600, "Invalid request"));
        };
        if !request.is_object() || request["jsonrpc"] != "2.0" || !valid_id {
            return Some(error(Value::Null, -32600, "Invalid request"));
        }
        if request
            .get("params")
            .is_some_and(|params| !params.is_object())
        {
            return id.map(|id| error(id, -32602, "Parameters must be an object"));
        }
        let Some(id) = id else {
            if method == "notifications/initialized" && self.initialized {
                self.ready = true;
            }
            return None;
        };
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
        let result = match method {
            "initialize" => {
                if self.initialized {
                    return Some(error(id, -32600, "Already initialized"));
                }
                if !params["protocolVersion"].is_string()
                    || !params["capabilities"].is_object()
                    || !params["clientInfo"]["name"].is_string()
                    || !params["clientInfo"]["version"].is_string()
                {
                    return Some(error(id, -32602, "Invalid initialize parameters"));
                }
                self.initialized = true;
                json!({"protocolVersion":VERSION,"capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"modn-mcp","version":env!("CARGO_PKG_VERSION")},"instructions":"Local scoped Modn CLI tools. Writes need server opt-in and explicit request approval. Previews never download."})
            }
            "ping" => json!({}),
            _ if !self.ready => {
                return Some(error(id, -32600, "Initialization must complete first"))
            }
            "tools/list" => {
                if !params.as_object().is_some_and(|object| object.is_empty()) {
                    return Some(error(id, -32602, "No pagination cursor is supported"));
                }
                tools::list(config.allow_writes)
            }
            "tools/call" => {
                let Some(name) = params["name"].as_str() else {
                    return Some(error(id, -32602, "Tool name is required"));
                };
                if !tools::known(name, config.allow_writes) {
                    return Some(error(id, -32602, "Unknown or disabled tool"));
                }
                if params.as_object().is_none_or(|object| {
                    object
                        .keys()
                        .any(|key| !matches!(key.as_str(), "name" | "arguments" | "_meta"))
                }) {
                    return Some(error(id, -32602, "Unknown call parameter"));
                }
                let input = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let args = match tools::arguments(config, name, input) {
                    Ok(args) => args,
                    Err(_) => {
                        return Some(error(
                            id,
                            -32602,
                            "Invalid tool arguments or disallowed path",
                        ))
                    }
                };
                tools::call(config, &args).await
            }
            _ => return Some(error(id, -32601, "Method not found")),
        };
        Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
    }
}
