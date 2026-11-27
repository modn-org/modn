use super::{not_implemented, SwitchArgs};
use anyhow::Result;
use modn_core::manager_roots::ManagerRoots;

pub(super) fn run(
    _args: &SwitchArgs,
    _roots: &ManagerRoots,
    _game: Option<&str>,
    _json: bool,
) -> Result<()> {
    Err(not_implemented("profile switch"))
}
