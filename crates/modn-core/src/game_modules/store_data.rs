//! Store identities that a game module declares as data in `module.json`.

use crate::stores::Storefront;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// Store identities of one game, from the optional `stores` object.
///
/// Rust matches these identities against the store libraries. Lua never
/// receives them and gains no filesystem or process access from them.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct StoreIdentities {
    /// Steam app ID, such as `1091500`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steam: Option<String>,
    /// GOG product ID, such as `1423049311`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gog: Option<String>,
    /// Epic Games Launcher `AppName`, such as `Ginger`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epic: Option<String>,
}

impl StoreIdentities {
    /// Declared identity for `store`, if any.
    #[must_use]
    pub fn get(&self, store: Storefront) -> Option<&str> {
        match store {
            Storefront::Steam => self.steam.as_deref(),
            Storefront::Gog => self.gog.as_deref(),
            Storefront::Epic => self.epic.as_deref(),
        }
    }

    pub(super) fn validate(&self) -> Result<()> {
        for (store, value) in [
            (Storefront::Steam, &self.steam),
            (Storefront::Gog, &self.gog),
            (Storefront::Epic, &self.epic),
        ] {
            let Some(value) = value else {
                continue;
            };
            if !crate::stores::valid_store_id(store, value) {
                bail!(
                    "MODULE_STORE_ID_INVALID: stores.{} has an invalid identity {value:?}",
                    store.as_str()
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::game_modules::check_module;
    use std::fs;

    fn check(stores: &str) -> anyhow::Result<()> {
        let temp = tempfile::tempdir().expect("temp");
        fs::write(
            temp.path().join("module.json"),
            format!(
                r#"{{"game_id":"store-game","version":"0.1.0","api_version":1,"entrypoint":"main.lua","stores":{stores}}}"#
            ),
        )
        .expect("manifest");
        fs::write(
            temp.path().join("main.lua"),
            r#"return { game_id = "store-game", api_version = mods_host.api_version }"#,
        )
        .expect("script");
        check_module(temp.path()).map(|_| ())
    }

    #[test]
    fn accepts_well_formed_store_identities() {
        check(r#"{"steam":"1091500","gog":"1423049311","epic":"Ginger"}"#).expect("valid");
        check("{}").expect("empty stores");
    }

    #[test]
    fn refuses_malformed_or_unknown_store_identities() {
        for stores in [
            r#"{"steam":"10915a0"}"#,
            r#"{"steam":""}"#,
            r#"{"gog":"-1"}"#,
            r#"{"epic":"Ginger launch"}"#,
        ] {
            let error = format!("{:#}", check(stores).expect_err(stores));
            assert!(error.starts_with("MODULE_STORE_ID_INVALID:"), "{error}");
        }
        let error = format!("{:#}", check(r#"{"itch":"1"}"#).expect_err("unknown store"));
        assert!(error.contains("unknown field"), "{error}");
    }
}
