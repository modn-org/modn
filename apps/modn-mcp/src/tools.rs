use crate::backend;
use crate::config::{identifier, Config};
use anyhow::{bail, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::ffi::OsString;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    query: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Archive {
    archive: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Install {
    archive: String,
    game_path: String,
    game_id: String,
    #[serde(default)]
    approved: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Uninstall {
    package: String,
    game_path: String,
    game_id: String,
    #[serde(default)]
    approved: bool,
}

fn schema(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}

pub fn list(allow_writes: bool) -> Value {
    let string = json!({"type":"string","minLength":1,"maxLength":4096});
    let mut tools = vec![
        (
            "catalog_search",
            "Search the local main catalog. No fetch.",
            schema(json!({"query":string}), &[]),
        ),
        (
            "registered_games",
            "List registered game installations.",
            schema(json!({}), &[]),
        ),
        (
            "inspect_archive",
            "Inspect archive metadata without extracting it.",
            schema(json!({"archive":string}), &["archive"]),
        ),
        (
            "preview_install",
            "Preview a local archive. No downloads or game writes.",
            schema(
                json!({"archive":string,"game_path":string,"game_id":string}),
                &["archive", "game_path", "game_id"],
            ),
        ),
        (
            "preview_uninstall",
            "Preview managed package removal.",
            schema(
                json!({"package":string,"game_path":string,"game_id":string}),
                &["package", "game_path", "game_id"],
            ),
        ),
    ];
    if allow_writes {
        tools.push(("install_archive", "Install a local archive only with approved=true. CLI permissions still apply.", schema(json!({"archive":string,"game_path":string,"game_id":string,"approved":{"type":"boolean","const":true}}), &["archive","game_path","game_id","approved"])));
        tools.push(("uninstall_package", "Remove a managed package only with approved=true. Ownership checks still apply.", schema(json!({"package":string,"game_path":string,"game_id":string,"approved":{"type":"boolean","const":true}}), &["package","game_path","game_id","approved"])));
    }
    let tools: Vec<_> = tools.into_iter().map(|(name, description, input_schema)| json!({"name":name,"description":description,"inputSchema":input_schema,"annotations":{"readOnlyHint":false,"destructiveHint":true,"idempotentHint":false,"openWorldHint":false}})).collect();
    json!({"tools":tools})
}

pub fn known(name: &str, writes: bool) -> bool {
    matches!(
        name,
        "catalog_search"
            | "registered_games"
            | "inspect_archive"
            | "preview_install"
            | "preview_uninstall"
    ) || (writes && matches!(name, "install_archive" | "uninstall_package"))
}

pub fn arguments(config: &Config, name: &str, input: Value) -> Result<Vec<OsString>> {
    if !input.is_object() {
        bail!("tool arguments must be an object");
    }
    if name.starts_with("preview_") && input.get("approved").is_some() {
        bail!("preview does not accept approval");
    }
    let mut args: Vec<OsString> = Vec::new();
    match name {
        "catalog_search" => {
            let search: Search = serde_json::from_value(input)?;
            args.push("search".into());
            if let Some(query) = search.query {
                if query.len() > 200
                    || query.starts_with('-')
                    || query.chars().any(char::is_control)
                {
                    bail!("invalid query");
                }
                args.push(query.into());
            }
        }
        "registered_games" => {
            let _: Empty = serde_json::from_value(input)?;
            args.extend(["game".into(), "list".into()]);
        }
        "inspect_archive" => {
            let archive: Archive = serde_json::from_value(input)?;
            args.extend([
                "inspect".into(),
                config.input_path(&archive.archive, false)?.into_os_string(),
            ]);
        }
        "preview_install" | "install_archive" => {
            let install: Install = serde_json::from_value(input)?;
            identifier(&install.game_id)?;
            if install.game_id.contains('/') {
                bail!("game_id must be a single identifier");
            }
            let game = config.input_path(&install.game_path, true)?;
            let archive = config.input_path(&install.archive, false)?;
            let write = name == "install_archive";
            if write && !(config.allow_writes && install.approved) {
                bail!("write requires server opt-in and approved=true");
            }
            if !write && install.approved {
                bail!("preview does not accept approval");
            }
            args.extend([
                "--game".into(),
                install.game_id.into(),
                "install".into(),
                archive.into_os_string(),
                "--path".into(),
                game.into_os_string(),
                if write { "--yes" } else { "--dry-run" }.into(),
            ]);
        }
        "preview_uninstall" | "uninstall_package" => {
            let uninstall: Uninstall = serde_json::from_value(input)?;
            identifier(&uninstall.game_id)?;
            identifier(&uninstall.package)?;
            if uninstall.game_id.contains('/') {
                bail!("game_id must be a single identifier");
            }
            let game = config.input_path(&uninstall.game_path, true)?;
            let write = name == "uninstall_package";
            if write && !(config.allow_writes && uninstall.approved) {
                bail!("write requires server opt-in and approved=true");
            }
            if !write && uninstall.approved {
                bail!("preview does not accept approval");
            }
            args.extend([
                "--game".into(),
                uninstall.game_id.into(),
                "uninstall".into(),
                uninstall.package.into(),
                "--path".into(),
                game.into_os_string(),
                if write { "--yes" } else { "--dry-run" }.into(),
            ]);
        }
        _ => bail!("unknown tool"),
    }
    Ok(args)
}

/// What the stdio loop should do after one tool call.
pub enum Reply {
    Response(Value),
    /// The client cancelled before the commit point, so MCP sends no response.
    Cancelled,
}

/// Maps one backend completion to the client response.
pub fn reply(completion: backend::Completion) -> Reply {
    if completion.cancelled && !completion.committed {
        return Reply::Cancelled;
    }
    Reply::Response(match completion.result {
        Ok(value) => json!({"content":[{"type":"text","text":value.to_string()}],"isError":false}),
        Err(error) => json!({"content":[{"type":"text","text":format!("{error}")}],"isError":true}),
    })
}

pub async fn call(
    config: &Config,
    args: &[OsString],
    token: Option<Value>,
    notifications: tokio::sync::mpsc::UnboundedSender<Value>,
    cancellation: tokio::sync::mpsc::UnboundedReceiver<()>,
) -> Reply {
    let channels = backend::Channels {
        notifications,
        cancellation,
    };
    reply(backend::run(config, args, token, channels).await)
}
