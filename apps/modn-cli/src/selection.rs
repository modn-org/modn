use anyhow::{bail, Result};
use modn_core::detection::detect_installations;
use modn_core::discovery::{validate_installation_for, Candidate};
use modn_core::error::{coded, ErrorCode};
use modn_core::game_modules::ModuleSelection;
use modn_core::manager_roots::ManagerRoots;
use modn_core::state::Store;
use modn_core::stores::StoreLibraries;
use std::path::PathBuf;

pub(crate) fn choose_candidate(
    roots: &ManagerRoots,
    game: &str,
    path: Option<PathBuf>,
) -> Result<Candidate> {
    if let Some(path) = path {
        let selection = ModuleSelection::resolve(roots, game)?;
        return Ok(Candidate {
            path: validate_installation_for(&selection, &path)?,
            source: "manual".into(),
        });
    }
    let detection = detect_installations(roots, &StoreLibraries::system(), &[game.to_owned()]);
    match detection.confirmed.as_slice() {
        [detected] => Ok(Candidate {
            path: detected.path.clone(),
            source: detected.store.as_str().into(),
        }),
        [] => bail!("{game} not found; specify --path to its installation directory"),
        _ => bail!("multiple installations found; specify --path to select one"),
    }
}

pub(crate) fn selected_installation(
    roots: &ManagerRoots,
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
                return Err(not_selected(
                    "installation is registered to a different game; use its registered ID",
                ));
            }
            (Some(id), _) => id,
            (None, [id]) => id,
            (None, []) => {
                return Err(not_selected(
                    "unregistered installation; specify --game <game-id>",
                ))
            }
            (None, _) => {
                return Err(not_selected(
                    "installation is registered for multiple games; specify --game",
                ))
            }
        };
        let selection = ModuleSelection::resolve(roots, id)?;
        return Ok((
            id.into(),
            validate_installation_for(&selection, &canonical)?,
        ));
    }
    let store = store.ok_or_else(|| not_selected("no registered game database for preview"))?;
    let registrations = store.list()?;
    let matches: Vec<_> = registrations
        .into_iter()
        .filter(|entry| selected.is_none_or(|id| entry.game_id == id))
        .collect();
    match matches.as_slice() {
        [entry] => {
            let selection = ModuleSelection::resolve(roots, &entry.game_id)?;
            Ok((
                entry.game_id.clone(),
                validate_installation_for(&selection, &entry.path)?,
            ))
        }
        [] => Err(not_selected(
            "no matching game registered; run 'modn game add <game-id> --path <directory>'",
        )),
        _ => Err(not_selected(
            "multiple installations match; specify --path or --game",
        )),
    }
}

fn not_selected(message: &str) -> anyhow::Error {
    coded(ErrorCode::GameNotSelected, message)
}
