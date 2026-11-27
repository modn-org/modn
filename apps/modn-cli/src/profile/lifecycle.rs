use super::{not_implemented, CreateArgs, DeleteArgs};
use anyhow::Result;
use modn_core::manager_roots::ManagerRoots;

pub(super) fn create(
    _args: &CreateArgs,
    _roots: &ManagerRoots,
    _game: Option<&str>,
    _json: bool,
) -> Result<()> {
    Err(not_implemented("profile create"))
}

pub(super) fn delete(
    _args: &DeleteArgs,
    _roots: &ManagerRoots,
    _game: Option<&str>,
    _json: bool,
) -> Result<()> {
    Err(not_implemented("profile delete"))
}
