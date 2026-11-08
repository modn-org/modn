use crate::commands::{AuthCommand, NexusAuthCommand};
use anyhow::{bail, Context, Result};
use modn_core::acquisition::NexusApiKey;
use std::env;
use std::io::{self, BufRead};
#[cfg(test)]
use std::sync::Mutex;

pub(super) const ENV_VAR: &str = "MODN_NEXUS_API_KEY";
const SERVICE: &str = "modn";
const USER: &str = "nexus-api-key";

/// The platform credential store operations Modn needs.
pub(super) trait SecretStore {
    fn get(&self) -> Result<Option<String>>;
    fn set(&self, secret: &str) -> Result<()>;
    fn delete(&self) -> Result<()>;
}

/// The operating system credential store, backed by Windows Credential
/// Manager, macOS Keychain Services or Secret Service.
pub(super) struct KeyringStore {
    service: String,
    user: String,
}

impl KeyringStore {
    pub(super) fn modn() -> Self {
        Self::new(SERVICE, USER)
    }

    fn new(service: &str, user: &str) -> Self {
        Self {
            service: service.to_owned(),
            user: user.to_owned(),
        }
    }

    fn entry(&self) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service, &self.user)
            .context("the operating system credential store is unavailable")
    }
}

impl SecretStore for KeyringStore {
    fn get(&self) -> Result<Option<String>> {
        match self.entry()?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(anyhow::Error::new(error)
                .context("read the Nexus API key from the credential store")),
        }
    }

    fn set(&self, secret: &str) -> Result<()> {
        self.entry()?.set_password(secret).map_err(|error| {
            anyhow::Error::new(error).context("save the Nexus API key to the credential store")
        })
    }

    fn delete(&self) -> Result<()> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(anyhow::Error::new(error)
                .context("remove the Nexus API key from the credential store")),
        }
    }
}

/// An in-memory store for unit tests only.
#[cfg(test)]
#[derive(Default)]
pub(super) struct MemoryStore {
    secret: Mutex<Option<String>>,
}

#[cfg(test)]
impl MemoryStore {
    fn with_secret(secret: &str) -> Self {
        Self {
            secret: Mutex::new(Some(secret.to_owned())),
        }
    }
}

#[cfg(test)]
impl SecretStore for MemoryStore {
    fn get(&self) -> Result<Option<String>> {
        Ok(self
            .secret
            .lock()
            .expect("memory store lock is not poisoned")
            .clone())
    }

    fn set(&self, secret: &str) -> Result<()> {
        *self
            .secret
            .lock()
            .expect("memory store lock is not poisoned") = Some(secret.to_owned());
        Ok(())
    }

    fn delete(&self) -> Result<()> {
        *self
            .secret
            .lock()
            .expect("memory store lock is not poisoned") = None;
        Ok(())
    }
}

pub(super) fn run(command: &AuthCommand, json: bool) -> Result<()> {
    match command {
        AuthCommand::Nexus { command } => match command {
            NexusAuthCommand::Set => set(json),
            NexusAuthCommand::Clear => clear(json),
            NexusAuthCommand::Status => status(json),
        },
    }
}

fn set(json: bool) -> Result<()> {
    save_key(&KeyringStore::modn(), io::stdin().lock())?;
    if json {
        println!(
            "{}",
            serde_json::json!({"nexus_api_key": "set", "source": "credential_store"})
        );
    } else {
        println!("Nexus API key saved to the credential store.");
    }
    Ok(())
}

fn save_key(store: &dyn SecretStore, reader: impl BufRead) -> Result<()> {
    let key = read_key(reader)?;
    store.set(key.expose_secret())
}

fn clear(json: bool) -> Result<()> {
    clear_key(&KeyringStore::modn())?;
    if json {
        println!(
            "{}",
            serde_json::json!({"nexus_api_key": "cleared", "source": "credential_store"})
        );
    } else {
        println!("Nexus API key removed from the credential store.");
    }
    Ok(())
}

fn clear_key(store: &dyn SecretStore) -> Result<()> {
    store.delete()
}

fn status(json: bool) -> Result<()> {
    let status = key_status(&KeyringStore::modn(), |name| env::var(name).ok())?;
    print_status(status, json);
    Ok(())
}

fn key_status(store: &dyn SecretStore, env: impl Fn(&str) -> Option<String>) -> Result<KeyStatus> {
    if environment_key_from(env)?.is_some() {
        return Ok(KeyStatus::Environment);
    }
    if store_key(store)?.is_some() {
        return Ok(KeyStatus::Store);
    }
    Ok(KeyStatus::Missing)
}

fn print_status(status: KeyStatus, json: bool) {
    if json {
        let value = match status {
            KeyStatus::Environment => {
                serde_json::json!({"nexus_api_key": "set", "source": ENV_VAR})
            }
            KeyStatus::Store => {
                serde_json::json!({"nexus_api_key": "set", "source": "credential_store"})
            }
            KeyStatus::Missing => serde_json::json!({"nexus_api_key": "not_set"}),
        };
        println!("{value}");
    } else {
        println!("{}", status.line());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyStatus {
    Store,
    Environment,
    Missing,
}

impl KeyStatus {
    fn line(self) -> &'static str {
        match self {
            Self::Store => "Nexus API key: set (credential store)",
            Self::Environment => "Nexus API key: set (MODN_NEXUS_API_KEY)",
            Self::Missing => "Nexus API key: not set",
        }
    }
}

fn read_key(mut reader: impl BufRead) -> Result<NexusApiKey> {
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .context("read the Nexus API key from stdin")?;
    let value = line.trim();
    if value.is_empty() {
        bail!("no Nexus API key on stdin; pipe the key as the first line");
    }
    NexusApiKey::new(value).context("stdin does not contain a valid Nexus API key")
}

fn environment_key_from(env: impl Fn(&str) -> Option<String>) -> Result<Option<NexusApiKey>> {
    match env(ENV_VAR).filter(|value| !value.is_empty()) {
        Some(value) => Ok(Some(
            NexusApiKey::new(&value).context("MODN_NEXUS_API_KEY is not a valid Nexus API key")?,
        )),
        None => Ok(None),
    }
}

fn store_key(store: &dyn SecretStore) -> Result<Option<NexusApiKey>> {
    match store
        .get()
        .context("read the Nexus API key from the credential store")?
    {
        Some(value) if !value.is_empty() => Ok(Some(
            NexusApiKey::new(&value).context("the stored Nexus API key is invalid")?,
        )),
        _ => Ok(None),
    }
}

/// Resolve the key for later download wiring: a non-empty environment value
/// wins over the credential store.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "wired by the Nexus download task")
)]
pub(super) fn resolve_key(
    store: &dyn SecretStore,
    env: impl Fn(&str) -> Option<String>,
) -> Result<Option<NexusApiKey>> {
    match environment_key_from(env)? {
        Some(key) => Ok(Some(key)),
        None => store_key(store),
    }
}

#[cfg(test)]
mod tests;
