//! Resolution and validation of a module-declared content root.

use crate::game_modules::ContentRootSpec;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Read-only health of a module-declared content root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContentRootStatus {
    Healthy,
    Missing,
    NotDirectory,
    Linked,
    Moved,
    Unavailable,
}

/// Inspection result for a content root; reads only, never creates.
#[derive(Debug)]
pub(crate) struct ContentRootHealth {
    /// Declared or recorded path, whether or not it exists.
    pub(crate) path: PathBuf,
    /// Canonical readable root, only when the status is healthy.
    pub(crate) root: Option<PathBuf>,
    pub(crate) status: ContentRootStatus,
    /// Moved target or base failure, when it adds detail.
    pub(crate) detail: Option<String>,
}

impl ContentRootHealth {
    /// Diagnostic code, message and next safe action for a problem.
    pub(crate) fn diagnostic(&self) -> Option<(&'static str, String, String)> {
        let path = self.path.display();
        let (code, message, action) = match self.status {
            ContentRootStatus::Healthy => return None,
            ContentRootStatus::Missing => (
                "CONTENT_ROOT_MISSING",
                format!("The module content root {path} is missing."),
                "Install a content package with Modn, or restore the folder, then run `modn doctor` again."
                    .into(),
            ),
            ContentRootStatus::NotDirectory => (
                "CONTENT_ROOT_NOT_DIRECTORY",
                format!("The module content root {path} is not a folder."),
                "Restore the declared folder or remove the file that occupies its path, then run `modn doctor` again."
                    .into(),
            ),
            ContentRootStatus::Linked => (
                "CONTENT_ROOT_LINKED",
                format!("The module content root {path} is a link or junction; Modn does not follow it."),
                "Replace the link with a real folder at the declared path, then run `modn doctor` again."
                    .into(),
            ),
            ContentRootStatus::Moved => (
                "CONTENT_ROOT_MOVED",
                match &self.detail {
                    Some(detail) => format!(
                        "The module content root {path} no longer resolves to its recorded path: {detail}"
                    ),
                    None => format!(
                        "The module content root {path} no longer resolves to its recorded path."
                    ),
                },
                "Move the folder back to its recorded path, then run `modn doctor` again."
                    .into(),
            ),
            ContentRootStatus::Unavailable => (
                "CONTENT_ROOT_UNAVAILABLE",
                match &self.detail {
                    Some(detail) => format!("The module content root {path} is unavailable: {detail}"),
                    None => format!("The module content root {path} is unavailable."),
                },
                "Restore the appdata folder and its permissions, then run `modn doctor` again."
                    .into(),
            ),
        };
        Some((code, message, action))
    }
}

/// Inspect a declared content root without writing or following links.
///
/// A recorded root is authoritative: Modn fails closed if it is missing,
/// is not a folder, became a link, or canonicalizes elsewhere. Without a
/// record, the declared path below `appdata` is inspected instead.
pub(crate) fn inspect(
    game: &Path,
    spec: &ContentRootSpec,
    appdata: &Path,
    recorded: Option<&Path>,
) -> ContentRootHealth {
    match recorded {
        Some(recorded) => inspect_recorded(game, recorded),
        None => inspect_declared(game, spec, appdata),
    }
}

fn inspect_recorded(game: &Path, recorded: &Path) -> ContentRootHealth {
    let unavailable = |detail: String| ContentRootHealth {
        path: recorded.to_owned(),
        root: None,
        status: ContentRootStatus::Unavailable,
        detail: Some(detail),
    };
    let metadata = match fs::symlink_metadata(recorded) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ContentRootHealth {
                path: recorded.to_owned(),
                root: None,
                status: ContentRootStatus::Missing,
                detail: None,
            }
        }
        Err(error) => return unavailable(error.to_string()),
    };
    if metadata.file_type().is_symlink() {
        return ContentRootHealth {
            path: recorded.to_owned(),
            root: None,
            status: ContentRootStatus::Linked,
            detail: None,
        };
    }
    if !metadata.is_dir() {
        return ContentRootHealth {
            path: recorded.to_owned(),
            root: None,
            status: ContentRootStatus::NotDirectory,
            detail: None,
        };
    }
    let resolved = match recorded.canonicalize() {
        Ok(resolved) => resolved,
        Err(error) => return unavailable(error.to_string()),
    };
    if resolved != recorded {
        return ContentRootHealth {
            path: recorded.to_owned(),
            root: None,
            status: ContentRootStatus::Moved,
            detail: Some(format!(
                "recorded {}, resolved {}",
                recorded.display(),
                resolved.display()
            )),
        };
    }
    if same_or_nested(&resolved, game) {
        return unavailable("it overlaps the game installation".into());
    }
    ContentRootHealth {
        path: recorded.to_owned(),
        root: Some(resolved),
        status: ContentRootStatus::Healthy,
        detail: None,
    }
}

fn inspect_declared(game: &Path, spec: &ContentRootSpec, appdata: &Path) -> ContentRootHealth {
    let relative = Path::new(&spec.path);
    let expected = appdata.join(relative);
    let unavailable = |detail: String| ContentRootHealth {
        path: expected.clone(),
        root: None,
        status: ContentRootStatus::Unavailable,
        detail: Some(detail),
    };
    if !appdata.is_absolute() {
        return unavailable("the appdata base is not absolute".into());
    }
    let metadata = match fs::symlink_metadata(appdata) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return unavailable("the appdata base is missing".into())
        }
        Err(error) => return unavailable(error.to_string()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return unavailable("the appdata base is not a real directory".into());
    }
    let base = match appdata.canonicalize() {
        Ok(base) => base,
        Err(error) => return unavailable(error.to_string()),
    };
    let expected = base.join(relative);
    let mut current = base;
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return ContentRootHealth {
                path: expected,
                root: None,
                status: ContentRootStatus::Unavailable,
                detail: Some("the declared path is not a safe relative path".into()),
            };
        };
        current.push(part);
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return ContentRootHealth {
                    path: expected,
                    root: None,
                    status: ContentRootStatus::Missing,
                    detail: None,
                }
            }
            Err(error) => {
                return ContentRootHealth {
                    path: expected,
                    root: None,
                    status: ContentRootStatus::Unavailable,
                    detail: Some(error.to_string()),
                }
            }
        };
        if metadata.file_type().is_symlink() {
            return ContentRootHealth {
                path: expected,
                root: None,
                status: ContentRootStatus::Linked,
                detail: Some(format!("{} is a link", current.display())),
            };
        }
        if !metadata.is_dir() {
            return ContentRootHealth {
                path: expected,
                root: None,
                status: ContentRootStatus::NotDirectory,
                detail: Some(format!("{} is not a folder", current.display())),
            };
        }
    }
    let resolved = match current.canonicalize() {
        Ok(resolved) => resolved,
        Err(error) => {
            return ContentRootHealth {
                path: expected,
                root: None,
                status: ContentRootStatus::Unavailable,
                detail: Some(error.to_string()),
            }
        }
    };
    if same_or_nested(&resolved, game) {
        return ContentRootHealth {
            path: expected,
            root: None,
            status: ContentRootStatus::Unavailable,
            detail: Some("it overlaps the game installation".into()),
        };
    }
    ContentRootHealth {
        path: expected,
        root: Some(resolved),
        status: ContentRootStatus::Healthy,
        detail: None,
    }
}

/// Resolve the production known-folder base for an `appdata` content root.
///
/// `MODN_APPDATA` is a test-only override; it is compiled out of release
/// builds and never read on behalf of an API 1 module.
pub fn appdata_directory() -> Result<PathBuf> {
    #[cfg(any(test, feature = "test-instruction-budget"))]
    if let Some(path) = std::env::var_os("MODN_APPDATA") {
        return Ok(PathBuf::from(path));
    }
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .context("APPDATA is not set; cannot resolve the module content root")
}

/// Canonical base and the validated content root below it.
///
/// The returned root may not exist yet. Callers create it only inside a
/// managed transaction and only below the returned base.
pub(crate) fn resolve(
    game: &Path,
    spec: &ContentRootSpec,
    appdata: &Path,
) -> Result<(PathBuf, PathBuf)> {
    if !appdata.is_absolute() {
        bail!(
            "appdata base must be an absolute path: {}",
            appdata.display()
        );
    }
    let base = fs::symlink_metadata(appdata)
        .with_context(|| format!("cannot inspect appdata base {}", appdata.display()))?;
    if !base.is_dir() || base.file_type().is_symlink() {
        bail!(
            "appdata base must be a real directory: {}",
            appdata.display()
        );
    }
    let base = appdata.canonicalize()?;
    let relative = Path::new(&spec.path);
    if relative.is_absolute() || relative.components().count() == 0 {
        bail!("content root path must be relative: {:?}", spec.path);
    }
    let mut current = base.clone();
    let mut missing = false;
    for component in relative.components() {
        let Component::Normal(part) = component else {
            bail!("content root path must not contain parent or prefix components");
        };
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    bail!(
                        "content root must not contain a link: {}",
                        current.display()
                    );
                }
                if !metadata.is_dir() {
                    bail!(
                        "content root component is not a directory: {}",
                        current.display()
                    );
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing = true;
            }
            Err(error) => return Err(error.into()),
        }
    }
    let root = if missing {
        base.join(relative)
    } else {
        current.canonicalize()?
    };
    if same_or_nested(&root, game) {
        bail!(
            "content root {} overlaps the game installation {}",
            root.display(),
            game.display()
        );
    }
    if root.parent().is_none() {
        bail!("content root must not be a drive or filesystem root");
    }
    Ok((base, root))
}

/// Revalidate a resolved content root, including link and overlap checks.
pub(crate) fn validate(game: &Path, base: &Path, root: &Path) -> Result<PathBuf> {
    let relative = root
        .strip_prefix(base)
        .context("content root is not below its appdata base")?;
    let spec = ContentRootSpec {
        base: "appdata".into(),
        path: relative
            .components()
            .map(|part| match part {
                Component::Normal(name) => name.to_str().map(str::to_owned),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
            .context("invalid content root path")?
            .join("/"),
    };
    let (_, resolved) = resolve(game, &spec, base)?;
    if resolved != root {
        bail!(
            "content root {} now resolves to {}; it moved or a parent became a link",
            root.display(),
            resolved.display()
        );
    }
    Ok(resolved)
}

fn same_or_nested(left: &Path, right: &Path) -> bool {
    let left = crate::transactions::ownership_key(&left.to_string_lossy());
    let right = crate::transactions::ownership_key(&right.to_string_lossy());
    left == right
        || left.starts_with(&format!("{right}/"))
        || right.starts_with(&format!("{left}/"))
        || left.starts_with(&format!("{right}\\"))
        || right.starts_with(&format!("{left}\\"))
}

#[cfg(test)]
#[path = "content_root_tests.rs"]
mod tests;
