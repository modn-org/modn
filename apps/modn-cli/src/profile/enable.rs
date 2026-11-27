use super::{not_implemented, EnableArgs};
use anyhow::Result;
use modn_core::manager_roots::ManagerRoots;

pub(super) fn enable(
    _args: &EnableArgs,
    _roots: &ManagerRoots,
    _game: Option<&str>,
    _json: bool,
) -> Result<()> {
    Err(not_implemented("profile enable"))
}

pub(super) fn disable(
    _args: &EnableArgs,
    _roots: &ManagerRoots,
    _game: Option<&str>,
    _json: bool,
) -> Result<()> {
    Err(not_implemented("profile disable"))
}
