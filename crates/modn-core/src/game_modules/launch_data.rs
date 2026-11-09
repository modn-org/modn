//! Launch targets that a game module declares as data in `module.json`.
//!
//! Rust validates every target before it opens a store URI or starts an
//! executable. Lua never receives launch targets or a process API.

use super::store_data::StoreIdentities;
use crate::stores::Storefront;
use anyhow::Result;
use serde::{Deserialize, Serialize};

const MAX_EXECUTABLE_BYTES: usize = 1024;
const MAX_URI_BYTES: usize = 2048;

/// Launch targets of one game, from the optional `launch` object.
///
/// A store URI is used only when detection shows that the selected folder came
/// from the same store. The executable is the fallback.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct LaunchTargets {
    /// Steam launch URI, such as `steam://rungameid/1091500`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steam: Option<String>,
    /// GOG Galaxy launch URI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gog: Option<String>,
    /// Epic Games Launcher launch URI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epic: Option<String>,
    /// Game-relative executable path, such as `bin/x64/Cyberpunk2077.exe`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable: Option<String>,
}

impl LaunchTargets {
    /// Declared store URI for `store`, if any.
    #[must_use]
    pub fn uri(&self, store: Storefront) -> Option<&str> {
        match store {
            Storefront::Steam => self.steam.as_deref(),
            Storefront::Gog => self.gog.as_deref(),
            Storefront::Epic => self.epic.as_deref(),
        }
    }

    /// Declared game-relative executable path, if any.
    #[must_use]
    pub fn executable(&self) -> Option<&str> {
        self.executable.as_deref()
    }

    /// Whether no target is declared.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steam.is_none()
            && self.gog.is_none()
            && self.epic.is_none()
            && self.executable.is_none()
    }

    pub(super) fn validate(&self, stores: &StoreIdentities) -> Result<()> {
        if self.is_empty() {
            return Err(invalid("launch must declare at least one target"));
        }
        for (store, uri) in [
            (Storefront::Steam, &self.steam),
            (Storefront::Gog, &self.gog),
            (Storefront::Epic, &self.epic),
        ] {
            let Some(uri) = uri else {
                continue;
            };
            validate_store_uri(store, uri, stores)?;
        }
        if let Some(executable) = &self.executable {
            validate_executable(executable)?;
        }
        Ok(())
    }
}

fn invalid(message: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!("MODULE_LAUNCH_TARGET_INVALID: {message}")
}

fn validate_store_uri(store: Storefront, uri: &str, stores: &StoreIdentities) -> Result<()> {
    if uri.len() > MAX_URI_BYTES {
        return Err(invalid(format!(
            "launch.{} is longer than {MAX_URI_BYTES} bytes",
            store.as_str()
        )));
    }
    if uri.chars().any(|ch| ch.is_control() || ch.is_whitespace()) {
        return Err(invalid(format!(
            "launch.{} must not contain whitespace or control characters",
            store.as_str()
        )));
    }
    if stores.get(store).is_none() {
        return Err(invalid(format!(
            "launch.{} requires stores.{} so detection can match the folder",
            store.as_str(),
            store.as_str()
        )));
    }
    let (scheme, rest) = uri
        .split_once(':')
        .ok_or_else(|| invalid(format!("launch.{} must be an absolute URI", store.as_str())))?;
    let expected = match store {
        Storefront::Steam => "steam",
        Storefront::Gog => "goggalaxy",
        Storefront::Epic => "com.epicgames.launcher",
    };
    if scheme != expected {
        return Err(invalid(format!(
            "launch.{} must use the {expected}: scheme, not {scheme}:",
            store.as_str()
        )));
    }
    if store == Storefront::Steam {
        let app_id = rest
            .strip_prefix("//rungameid/")
            .ok_or_else(|| invalid("launch.steam must have the form steam://rungameid/<app id>"))?;
        if app_id.is_empty() || !app_id.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(invalid(
                "launch.steam must have the form steam://rungameid/<app id>",
            ));
        }
        if stores.get(store) != Some(app_id) {
            return Err(invalid("launch.steam app id must equal stores.steam"));
        }
    } else if rest.is_empty() || rest == "//" {
        return Err(invalid(format!(
            "launch.{} must name a launch action",
            store.as_str()
        )));
    }
    Ok(())
}

fn validate_executable(executable: &str) -> Result<()> {
    if executable.len() > MAX_EXECUTABLE_BYTES {
        return Err(invalid(format!(
            "launch.executable is longer than {MAX_EXECUTABLE_BYTES} bytes"
        )));
    }
    let normalized = crate::archive::validated_name(executable)
        .map_err(|error| invalid(format!("launch.executable is unsafe: {error:#}")))?;
    if normalized != executable {
        return Err(invalid(
            "launch.executable must be a normalized game-relative path",
        ));
    }
    if !executable.to_ascii_lowercase().ends_with(".exe") || executable.len() <= 4 {
        return Err(invalid("launch.executable must name an .exe file"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::game_modules::check_module;
    use std::fs;

    fn check(launch: &str, stores: &str) -> anyhow::Result<()> {
        let temp = tempfile::tempdir().expect("temp");
        fs::write(
            temp.path().join("module.json"),
            format!(
                r#"{{"game_id":"launch-game","version":"0.1.0","api_version":1,"entrypoint":"main.lua","stores":{stores},"launch":{launch}}}"#
            ),
        )
        .expect("manifest");
        fs::write(
            temp.path().join("main.lua"),
            r#"return { game_id = "launch-game", api_version = mods_host.api_version }"#,
        )
        .expect("script");
        check_module(temp.path()).map(|_| ())
    }

    fn error(launch: &str, stores: &str) -> String {
        format!(
            "{:#}",
            check(launch, stores).expect_err("launch target must be refused")
        )
    }

    #[test]
    fn accepts_store_uris_and_a_relative_executable() {
        check(
            r#"{"steam":"steam://rungameid/1091500","executable":"bin/x64/Game.exe"}"#,
            r#"{"steam":"1091500"}"#,
        )
        .expect("valid launch targets");
        check(r#"{"executable":"Game.exe"}"#, "{}").expect("executable only");
    }

    #[test]
    fn refuses_absolute_and_traversing_executables() {
        for executable in [
            "C:/game/Game.exe",
            "/game/Game.exe",
            "..\\Game.exe",
            "bin/../Game.exe",
            "bin/../../Game.exe",
            "bin\\Game.exe",
        ] {
            let launch = serde_json::json!({"executable": executable}).to_string();
            let message = error(&launch, "{}");
            assert!(
                message.starts_with("MODULE_LAUNCH_TARGET_INVALID:"),
                "{executable}: {message}"
            );
        }
    }

    #[test]
    fn refuses_non_executable_files() {
        for executable in ["Game.dll", "bin/Game", "Game.exe.txt", ".exe"] {
            let launch = serde_json::json!({"executable": executable}).to_string();
            let message = error(&launch, "{}");
            assert!(
                message.starts_with("MODULE_LAUNCH_TARGET_INVALID:"),
                "{message}"
            );
        }
    }

    #[test]
    fn refuses_unknown_or_mismatched_store_uris() {
        for (launch, stores) in [
            (
                r#"{"steam":"https://example.test/game"}"#,
                r#"{"steam":"1091500"}"#,
            ),
            (
                r#"{"steam":"goggalaxy://openGameView/1"}"#,
                r#"{"steam":"1091500"}"#,
            ),
            (r#"{"gog":"steam://rungameid/1"}"#, r#"{"gog":"1"}"#),
            (r#"{"steam":"steam://rungameid/1"}"#, "{}"),
            (
                r#"{"steam":"steam://rungameid/1"}"#,
                r#"{"steam":"1091500"}"#,
            ),
        ] {
            let message = error(launch, stores);
            assert!(
                message.starts_with("MODULE_LAUNCH_TARGET_INVALID:"),
                "{message}"
            );
        }
        let message = error(r#"{"itch":"itch://game"}"#, "{}");
        assert!(message.contains("unknown field"), "{message}");
    }

    #[test]
    fn refuses_an_empty_launch_object() {
        let message = error("{}", r#"{"steam":"1091500"}"#);
        assert!(
            message.starts_with("MODULE_LAUNCH_TARGET_INVALID:"),
            "{message}"
        );
    }
}
