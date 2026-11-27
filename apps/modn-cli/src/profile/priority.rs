use super::{not_implemented, PriorityArgs};
use anyhow::Result;
use modn_core::manager_roots::ManagerRoots;

pub(super) fn run(
    _args: &PriorityArgs,
    _roots: &ManagerRoots,
    _game: Option<&str>,
    _json: bool,
) -> Result<()> {
    Err(not_implemented("profile priority"))
}
