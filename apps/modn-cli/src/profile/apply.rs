use super::{not_implemented, ApplyArgs};
use anyhow::Result;
use modn_core::manager_roots::ManagerRoots;

pub(super) fn run(
    _args: &ApplyArgs,
    _roots: &ManagerRoots,
    _game: Option<&str>,
    _json: bool,
) -> Result<()> {
    Err(not_implemented("profile apply"))
}
