use super::{not_implemented, ListArgs};
use anyhow::Result;
use modn_core::manager_roots::ManagerRoots;

pub(super) fn run(
    _args: &ListArgs,
    _roots: &ManagerRoots,
    _game: Option<&str>,
    _json: bool,
) -> Result<()> {
    Err(not_implemented("profile list"))
}
