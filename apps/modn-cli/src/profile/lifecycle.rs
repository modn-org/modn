use super::{open_installation, print_json, CreateArgs, DeleteArgs, ProfileView};
use crate::state_access::InstallationAccess;
use anyhow::Result;
use modn_core::error::{coded, ErrorCode};
use modn_core::manager_roots::ManagerRoots;
use modn_core::profiles;

pub(super) fn create(
    args: &CreateArgs,
    roots: &ManagerRoots,
    game: Option<&str>,
    json: bool,
) -> Result<()> {
    let selected = open_installation(roots, args.path.clone(), game, InstallationAccess::Write)?;
    let mut store = selected.store.expect("write access opens manager state");
    let profile = profiles::create(
        &mut store,
        &selected.root,
        &args.name,
        args.clone.as_deref(),
    )?;
    let view = ProfileView::new(&profile);
    if json {
        print_json(&view)?;
        return Ok(());
    }
    let source = match args.clone.as_deref() {
        Some(source) => format!(" from {source:?}"),
        None => String::new(),
    };
    let packages = match view.packages.len() {
        1 => "1 package".to_owned(),
        count => format!("{count} packages"),
    };
    println!(
        "Created inactive profile {:?}{source} with {packages}.",
        view.name
    );
    Ok(())
}

pub(super) fn delete(
    args: &DeleteArgs,
    roots: &ManagerRoots,
    game: Option<&str>,
    json: bool,
) -> Result<()> {
    if !args.yes {
        return Err(coded(
            ErrorCode::ApprovalRequired,
            format!(
                "deleting profile {:?} changes manager state; rerun with --yes",
                args.name
            ),
        ));
    }
    let selected = open_installation(roots, args.path.clone(), game, InstallationAccess::Write)?;
    let mut store = selected.store.expect("write access opens manager state");
    profiles::delete(&mut store, &selected.root, &args.name)?;
    if json {
        print_json(&serde_json::json!({"deleted": args.name}))?;
    } else {
        println!("Deleted profile {:?}.", args.name);
    }
    Ok(())
}
