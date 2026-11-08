use super::*;
use std::io::Cursor;

const KEY: &str = "NEXUS_STATUS_SECRET_123";

#[test]
fn memory_store_sets_gets_and_clears() {
    let store = MemoryStore::default();
    assert_eq!(store.get().expect("empty store"), None);
    store.set(KEY).expect("set");
    assert_eq!(store.get().expect("stored key").as_deref(), Some(KEY));
    store.delete().expect("clear");
    assert_eq!(store.get().expect("cleared store"), None);
}

#[test]
fn set_and_clear_work_against_the_memory_store() {
    let store = MemoryStore::default();
    save_key(&store, Cursor::new(format!("  {KEY}  \r\nsecond line\n"))).expect("save");
    assert_eq!(store.get().expect("stored key").as_deref(), Some(KEY));
    clear_key(&store).expect("clear");
    assert_eq!(store.get().expect("cleared store"), None);
}

#[test]
fn empty_stdin_does_not_touch_the_store() {
    let store = MemoryStore::default();
    assert!(save_key(&store, Cursor::new("   \r\n")).is_err());
    assert_eq!(store.get().expect("unchanged store"), None);
}

#[test]
fn environment_key_wins_over_the_store() {
    let store = MemoryStore::with_secret("store-key");
    let key = resolve_key(&store, |name| {
        (name == ENV_VAR).then(|| "env-key".to_owned())
    })
    .expect("resolve")
    .expect("key");
    assert_eq!(key.expose_secret(), "env-key");
}

#[test]
fn an_empty_environment_value_falls_back_to_the_store() {
    let store = MemoryStore::with_secret("store-key");
    let key = resolve_key(&store, |_| Some(String::new()))
        .expect("resolve")
        .expect("key");
    assert_eq!(key.expose_secret(), "store-key");
}

#[test]
fn no_environment_value_and_no_stored_key_resolve_to_none() {
    let store = MemoryStore::default();
    assert!(resolve_key(&store, |_| None).expect("resolve").is_none());
}

#[test]
fn status_uses_the_memory_store_and_never_prints_the_key() {
    let store = MemoryStore::with_secret(KEY);
    let status = key_status(&store, |_| None).expect("status");
    assert_eq!(status, KeyStatus::Store);
    assert_eq!(status.line(), "Nexus API key: set (credential store)");
    assert!(!status.line().contains(KEY));

    let environment =
        key_status(&store, |name| (name == ENV_VAR).then(|| KEY.to_owned())).expect("status");
    assert_eq!(environment, KeyStatus::Environment);
    assert_eq!(
        environment.line(),
        "Nexus API key: set (MODN_NEXUS_API_KEY)"
    );
    assert!(!environment.line().contains(KEY));

    let missing = key_status(&MemoryStore::default(), |_| None).expect("status");
    assert_eq!(missing, KeyStatus::Missing);
    assert_eq!(missing.line(), "Nexus API key: not set");
}

#[test]
#[ignore = "uses the real operating system credential store"]
fn keyring_store_round_trips_a_throwaway_entry() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let service = format!("modn-test-{}-{unique}", std::process::id());
    let store = KeyringStore::new(&service, USER);
    store.set("throwaway-secret").expect("set");
    assert_eq!(
        store.get().expect("get").as_deref(),
        Some("throwaway-secret")
    );
    store.delete().expect("delete");
    assert_eq!(store.get().expect("get after delete"), None);
}
