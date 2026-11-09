use super::{directory_files, read_bounded, valid_store_id, StoreInstallation, Storefront};
use std::path::{Component, Path, PathBuf};

/// Games that one Steam folder and its extra libraries list.
///
/// `steamapps/libraryfolders.vdf` lists the libraries. Each library holds one
/// `steamapps/appmanifest_<app id>.acf` per game, whose `installdir` names
/// the folder below `steamapps/common`.
pub(super) fn installations(root: &Path) -> Vec<StoreInstallation> {
    let mut installations = Vec::new();
    for library in libraries(root) {
        for manifest in directory_files(&library.join("steamapps")) {
            let Some(app_id) = manifest_app_id(&manifest) else {
                continue;
            };
            let Some(text) = read_bounded(&manifest) else {
                continue;
            };
            let declared = value(&text, "appid");
            let Some(folder) = value(&text, "installdir") else {
                continue;
            };
            if declared.as_deref() != Some(app_id.as_str()) || !single_folder(&folder) {
                continue;
            }
            installations.push(StoreInstallation {
                store: Storefront::Steam,
                store_id: app_id,
                path: library.join("steamapps").join("common").join(folder),
            });
        }
    }
    installations
}

fn libraries(root: &Path) -> Vec<PathBuf> {
    let mut libraries = vec![root.to_path_buf()];
    if let Some(text) = read_bounded(&root.join("steamapps").join("libraryfolders.vdf")) {
        for (key, path) in pairs(&text) {
            let path = PathBuf::from(path);
            if key.eq_ignore_ascii_case("path") && path.is_absolute() && !libraries.contains(&path)
            {
                libraries.push(path);
            }
        }
    }
    libraries
}

fn manifest_app_id(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let app_id = name.strip_prefix("appmanifest_")?.strip_suffix(".acf")?;
    valid_store_id(Storefront::Steam, app_id).then(|| app_id.to_owned())
}

fn single_folder(name: &str) -> bool {
    let mut components = Path::new(name).components();
    name.len() <= 255
        && !name.contains(['/', '\\', ':'])
        && !name.chars().any(char::is_control)
        && matches!(components.next(), Some(Component::Normal(_)))
        && components.next().is_none()
}

fn value(text: &str, key: &str) -> Option<String> {
    pairs(text)
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .map(|(_, value)| value)
}

/// Key and value pairs of a Valve KeyValues text file, one pair per line.
///
/// Section names and braces stand on their own lines and are skipped.
fn pairs(text: &str) -> impl Iterator<Item = (String, String)> + '_ {
    text.lines()
        .filter_map(|line| match quoted(line).as_slice() {
            [key, value] => Some((key.clone(), value.clone())),
            _ => None,
        })
}

fn quoted(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut characters = line.chars();
    while let Some(character) = characters.next() {
        if character != '"' {
            continue;
        }
        let mut token = String::new();
        let mut closed = false;
        while let Some(character) = characters.next() {
            match character {
                '"' => {
                    closed = true;
                    break;
                }
                '\\' => token.extend(characters.next()),
                other => token.push(other),
            }
        }
        if !closed {
            return Vec::new();
        }
        tokens.push(token);
    }
    tokens
}
