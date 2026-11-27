use super::{open_installation, print_json, ListArgs, ProfileView};
use crate::state_access::InstallationAccess;
use anyhow::Result;
use modn_core::manager_roots::ManagerRoots;
use serde::Serialize;

/// The `profile list --json` document.
#[derive(Serialize)]
struct ProfileList<'a> {
    game: &'a str,
    path: String,
    profiles: Vec<ProfileView<'a>>,
}

pub(super) fn run(
    args: &ListArgs,
    roots: &ManagerRoots,
    game: Option<&str>,
    json: bool,
) -> Result<()> {
    let selected = open_installation(roots, args.path.clone(), game, InstallationAccess::Preview)?;
    let store = selected
        .store
        .as_ref()
        .expect("preview access requires manager state");
    let profiles = modn_core::profiles::list(store, &selected.root)?;
    let views = profiles.iter().map(ProfileView::new).collect::<Vec<_>>();
    if json {
        print_json(&ProfileList {
            game: &selected.game_id,
            path: selected.root.to_string_lossy().into_owned(),
            profiles: views,
        })?;
        return Ok(());
    }
    print_table(&views);
    Ok(())
}

fn print_table(profiles: &[ProfileView<'_>]) {
    if profiles.is_empty() {
        println!("No profiles recorded for this installation.");
        return;
    }
    let name_width = profiles
        .iter()
        .map(|profile| profile.name.len())
        .max()
        .unwrap_or(0)
        .max("PROFILE".len());
    println!("{:<name_width$}  ACTIVE  PACKAGES", "PROFILE");
    for profile in profiles {
        let active = if profile.active { "*" } else { "" };
        let packages = if profile.packages.is_empty() {
            "none".to_owned()
        } else {
            profile
                .packages
                .iter()
                .map(|package| format!("#{} {}", package.rank, package.package))
                .collect::<Vec<_>>()
                .join(", ")
        };
        println!("{:<name_width$}  {active:<6}  {packages}", profile.name);
    }
}
