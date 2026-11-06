//! Resolution and validation of a module-declared content root.

use crate::game_modules::ContentRootSpec;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Component, Path, PathBuf};

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
