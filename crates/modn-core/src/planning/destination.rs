//! Destination checks against the files that already exist on disk.

use std::fs;
use std::path::Path;

/// Why an existing destination cannot receive a planned file.
///
/// A `managed_file` destination holds a regular file that another package of
/// the active profile owns, so a plan may replace it instead of refusing. A
/// link, a directory or any other existing file still returns an issue.
pub(crate) fn existing_destination_issue(
    game: &Path,
    target: &str,
    managed_file: bool,
) -> Option<&'static str> {
    let mut current = game.to_path_buf();
    let canonical_game = game.canonicalize().ok()?;
    let components: Vec<_> = target.split('/').collect();
    for (index, component) in components.iter().enumerate() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if index + 1 < components.len() && !metadata.is_dir() {
                    return Some("NON_DIRECTORY_ANCESTOR");
                }
                if metadata.file_type().is_symlink() {
                    return Some("LINKED_DESTINATION");
                }
                let Ok(resolved) = current.canonicalize() else {
                    return Some("LINKED_DESTINATION");
                };
                if !resolved.starts_with(&canonical_game) {
                    return Some("LINKED_DESTINATION");
                }
                // On Windows, a short (8.3) name can alias a long directory name
                // without matching its SQLite ownership key. Never publish a
                // new file below an ancestor addressed through a different name.
                #[cfg(windows)]
                {
                    let key = crate::transactions::ownership_key(component);
                    if !resolved
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|actual| crate::transactions::ownership_key(actual) == key)
                    {
                        return Some("ALIASED_DESTINATION");
                    }
                    if component.contains('~')
                        && !current.parent().is_some_and(|parent| {
                            fs::read_dir(parent).is_ok_and(|entries| {
                                entries.flatten().any(|entry| {
                                    entry.file_name().to_str().is_some_and(|actual| {
                                        crate::transactions::ownership_key(actual) == key
                                    })
                                })
                            })
                        })
                    {
                        return Some("ALIASED_DESTINATION");
                    }
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => return Some("DESTINATION_UNREADABLE"),
        }
    }
    if let Ok(metadata) = fs::symlink_metadata(&current) {
        if managed_file && metadata.is_file() && !metadata.file_type().is_symlink() {
            return None;
        }
        Some("EXISTING_FILE")
    } else if game.join(target).exists() {
        Some("EXISTING_FILE")
    } else {
        None
    }
}
