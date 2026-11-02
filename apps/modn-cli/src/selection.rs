use anyhow::{bail, Result};
use modn_core::discovery::{discover_for, validate_installation_for, Candidate};
use modn_core::state::Store;
use std::path::PathBuf;

pub(crate) fn choose_candidate(game: &str, path: Option<PathBuf>) -> Result<Candidate> {
    if let Some(path) = path {
        return Ok(Candidate {
            path: validate_installation_for(game, &path)?,
            source: "manual".into(),
        });
    }
    let candidates = discover_for(game);
    match candidates.as_slice() {
        [candidate] => Ok(candidate.clone()),
        [] => bail!("{game} not found; specify --path to its installation directory"),
        _ => bail!("multiple installations found; specify --path to select one"),
    }
}

pub(crate) fn selected_installation(
    store: Option<&Store>,
    path: Option<PathBuf>,
    requested: Option<&str>,
) -> Result<(String, PathBuf)> {
    let selected = requested;
    if let Some(path) = path {
        let canonical = path.canonicalize()?;
        let registered = store.map(Store::list).transpose()?.unwrap_or_default();
        let ids: Vec<_> = registered
            .iter()
            .filter(|entry| {
                entry
                    .path
                    .canonicalize()
                    .is_ok_and(|registered_root| registered_root == canonical)
            })
            .map(|entry| entry.game_id.as_str())
            .collect();
        let id = match (selected, ids.as_slice()) {
            (Some(id), ids) if !ids.is_empty() && !ids.contains(&id) => {
                bail!("installation is registered to a different game; use its registered ID");
            }
            (Some(id), _) => id,
            (None, [id]) => id,
            (None, []) => bail!("unregistered installation; specify --game <game-id>"),
            (None, _) => bail!("installation is registered for multiple games; specify --game"),
        };
        return Ok((id.into(), validate_installation_for(id, &canonical)?));
    }
    let store = store.ok_or_else(|| anyhow::anyhow!("no registered game database for preview"))?;
    let registrations = store.list()?;
    let matches: Vec<_> = registrations
        .into_iter()
        .filter(|entry| selected.is_none_or(|id| entry.game_id == id))
        .collect();
    match matches.as_slice() {
        [entry] => Ok((
            entry.game_id.clone(),
            validate_installation_for(&entry.game_id, &entry.path)?,
        )),
        [] => {
            bail!("no matching game registered; run 'modn game add <game-id> --path <directory>'")
        }
        _ => bail!("multiple installations match; specify --path or --game"),
    }
}
