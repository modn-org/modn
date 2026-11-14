#![allow(
    dead_code,
    reason = "each integration-test crate uses a different subset of this fixture"
)]
use crate::catalog_authoring::{path_text, run};
use std::path::Path;

pub fn keygen(keys: &Path, name: &str) -> std::process::Output {
    run(&[
        "--json",
        "catalog",
        "keygen",
        "--out",
        path_text(keys),
        "--name",
        name,
    ])
}

pub fn sign(catalog: &Path, key: &Path) -> std::process::Output {
    run(&[
        "--json",
        "catalog",
        "sign",
        path_text(catalog),
        "--key",
        path_text(key),
    ])
}

pub fn verify(catalog: &Path, key: &Path) -> std::process::Output {
    run(&[
        "--json",
        "catalog",
        "verify",
        path_text(catalog),
        "--key",
        path_text(key),
    ])
}

