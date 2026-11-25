use crate::commands::ThemeCommand;
use anyhow::{bail, Context, Result};
use modn_core::manager_roots::ManagerRoots;
use modn_core::themes::{
    inspect_theme_dir, BucketThemeRoot, CssChecker, LightningChecker, RegisteredTheme, Resolved,
    SelectionStore, ThemeCatalog, ThemeError, ThemeManifest, ThemeProblem, ThemeRegistry,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

const THEME_INVALID: &str = "THEME_INVALID";

pub(super) fn run(
    command: &ThemeCommand,
    roots: &ManagerRoots,
    game: Option<&str>,
    json: bool,
) -> Result<()> {
    let checker = LightningChecker;
    if let ThemeCommand::Check { directory } = command {
        return check(directory, &checker, json);
    }
    let data = roots.profile();
    match command {
        ThemeCommand::Check { .. } => Ok(()),
        ThemeCommand::Register { directory } => register(directory, data, &checker, json),
        ThemeCommand::List => list(roots, game, &checker, json),
        ThemeCommand::Remove { theme_id } => remove(theme_id, data, game, json),
        ThemeCommand::Select {
            game,
            theme,
            variant,
            ..
        } => select(
            roots,
            game,
            theme.as_deref(),
            variant.as_deref(),
            &checker,
            json,
        ),
        ThemeCommand::Reset => reset(data, game, json),
        ThemeCommand::Resolve => resolve(roots, game, &checker, json),
    }
}

fn check(directory: &Path, checker: &dyn CssChecker, json: bool) -> Result<()> {
    match inspect_theme_dir(directory, checker) {
        Ok(theme) => {
            if json {
                print_value(&json!({
                    "ok": true,
                    "theme": entry(&theme.manifest, false),
                    "diagnostics": [],
                }))?;
            } else {
                println!(
                    "{} ({}): valid theme, {} variant(s).",
                    theme.manifest.name,
                    theme.manifest.game,
                    theme.manifest.variants.len()
                );
            }
            Ok(())
        }
        Err(problems) => {
            if json {
                print_value(&json!({
                    "ok": false,
                    "theme": null,
                    "diagnostics": problems,
                }))?;
            } else {
                println!("theme is not valid:");
                for problem in &problems {
                    println!("- {problem}");
                }
            }
            std::process::exit(1);
        }
    }
}

fn register(directory: &Path, data: &Path, checker: &dyn CssChecker, json: bool) -> Result<()> {
    match ThemeRegistry::new(data).register(directory, checker) {
        Ok(theme) => {
            let value = json!({
                "registered": registered_entry(&theme),
                "digest": format!("blake3:{}", theme.blake3()),
            });
            if json {
                print_value(&value)?;
            } else {
                println!(
                    "Registered {} ({}) with digest blake3:{}.",
                    theme.name(),
                    theme.game(),
                    theme.blake3()
                );
            }
            Ok(())
        }
        Err(ThemeError::Invalid(problems)) => refuse(&problems, json),
        Err(error) => Err(error.into()),
    }
}

fn list(
    roots: &ManagerRoots,
    game: Option<&str>,
    checker: &dyn CssChecker,
    json: bool,
) -> Result<()> {
    let data = roots.profile();
    let selections = SelectionStore::new(data).list()?;
    let output = catalog(roots, checker).list_output(game, &selections);
    if json {
        print_value(&serde_json::to_value(&output)?)?;
    } else {
        for theme in output.themes() {
            let selected = if theme.selected() { " (selected)" } else { "" };
            println!(
                "{}/{}: {} from {}{selected}",
                theme.game(),
                theme.id(),
                theme.name(),
                theme.source().label()
            );
        }
        for problem in output.diagnostics() {
            eprintln!("{problem}");
        }
    }
    Ok(())
}

fn remove(theme_id: &str, data: &Path, game: Option<&str>, json: bool) -> Result<()> {
    let removed = ThemeRegistry::new(data).remove(theme_id, game)?;
    let value = json!({
        "removed": {"id": removed.id(), "game": removed.game()},
    });
    if json {
        print_value(&value)?;
    } else {
        println!("Removed {} ({}).", removed.id(), removed.game());
    }
    Ok(())
}

fn select(
    roots: &ManagerRoots,
    game: &str,
    theme: Option<&str>,
    variant: Option<&str>,
    checker: &dyn CssChecker,
    json: bool,
) -> Result<()> {
    let data = roots.profile();
    let Some(theme) = theme else {
        let selection = SelectionStore::new(data).select_none(game)?;
        let value = json!({
            "selected": {
                "game": selection.game(),
                "theme": Value::Null,
                "variant": Value::Null,
            },
        });
        if json {
            print_value(&value)?;
        } else {
            println!("Selected no theme for {}.", selection.game());
        }
        return Ok(());
    };
    let catalog = catalog(roots, checker);
    let entry = catalog
        .entries()
        .iter()
        .find(|entry| entry.game() == game && entry.id() == theme)
        .with_context(|| format!("no theme \"{theme}\" for game \"{game}\" is available"))?;
    if let Some(variant) = variant {
        if !entry
            .inspected()
            .manifest
            .variants
            .iter()
            .any(|listed| listed == variant)
        {
            bail!("theme \"{theme}\" has no variant \"{variant}\"");
        }
    }
    let selection = SelectionStore::new(data).select(game, theme, variant)?;
    let value = json!({
        "selected": {
            "game": selection.game(),
            "theme": selection.theme(),
            "variant": selection.variant(),
        },
    });
    if json {
        print_value(&value)?;
    } else {
        println!("Selected {} for {}.", selection.theme(), selection.game());
    }
    Ok(())
}

fn reset(data: &Path, game: Option<&str>, json: bool) -> Result<()> {
    let store = SelectionStore::new(data);
    let games = match game {
        Some(game) => {
            if store.clear(game)? {
                vec![game.to_owned()]
            } else {
                Vec::new()
            }
        }
        None => store.clear_all()?,
    };
    if json {
        print_value(&json!({"reset": games}))?;
    } else if games.is_empty() {
        println!("No theme selections to reset.");
    } else {
        println!("Reset theme selections for {}.", games.join(", "));
    }
    Ok(())
}

fn resolve(
    roots: &ManagerRoots,
    game: Option<&str>,
    checker: &dyn CssChecker,
    json: bool,
) -> Result<()> {
    let Some(game) = game else {
        bail!("theme resolve requires --game <game-id>");
    };
    let selection = SelectionStore::new(roots.profile()).get(game)?;
    let resolved = catalog(roots, checker).resolve(game, selection.as_ref());
    if json {
        print_value(&serde_json::to_value(&resolved)?)?;
    } else {
        print_resolved(&resolved);
    }
    Ok(())
}

fn catalog(roots: &ManagerRoots, checker: &dyn CssChecker) -> ThemeCatalog {
    ThemeCatalog::build(
        &bucket_roots(roots.profile(), &roots.official_main()),
        &ThemeRegistry::new(roots.profile()),
        checker,
    )
}

fn bucket_roots(profile: &Path, official_main: &Path) -> Vec<BucketThemeRoot> {
    let mut roots = vec![BucketThemeRoot::official(official_main.to_path_buf())];
    roots.extend(registered_roots(profile));
    roots
}

fn registered_roots(profile: &Path) -> Vec<BucketThemeRoot> {
    let directory = profile.join("buckets").join("registrations");
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut roots = BTreeMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension() != Some(OsStr::new("json")) {
            continue;
        }
        let Some(id) = path.file_stem().and_then(OsStr::to_str) else {
            continue;
        };
        if id == "main" {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        let Some(root) = value.get("path").and_then(Value::as_str) else {
            continue;
        };
        roots.insert(id.to_owned(), PathBuf::from(root));
    }
    roots
        .into_iter()
        .map(|(id, root)| BucketThemeRoot::bucket(id, root))
        .collect()
}

fn entry(manifest: &ThemeManifest, selected: bool) -> Value {
    json!({
        "id": manifest.id,
        "game": manifest.game,
        "name": manifest.name,
        "theme_api": manifest.theme_api,
        "variants": manifest.variants,
        "icon": manifest.icon,
        "source": local_source(),
        "selected": selected,
    })
}

fn registered_entry(theme: &RegisteredTheme) -> Value {
    json!({
        "id": theme.id(),
        "game": theme.game(),
        "name": theme.name(),
        "theme_api": 1,
        "variants": theme.variants(),
        "icon": theme.icon(),
        "source": local_source(),
        "selected": false,
    })
}

fn local_source() -> Value {
    json!({"kind": "local", "bucket": null, "label": "local registry"})
}

fn refuse(problems: &[ThemeProblem], json: bool) -> Result<()> {
    let message = match problems.first() {
        Some(first) => format!("the theme failed validation: {first}"),
        None => "the theme failed validation".to_owned(),
    };
    if json {
        print_error(&json!({
            "error": THEME_INVALID,
            "message": message,
            "diagnostics": problems,
        }))?;
    } else {
        eprintln!("error: {message}");
        for problem in problems {
            eprintln!("- {problem}");
        }
    }
    std::process::exit(1);
}

fn print_value(value: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn print_error(value: &Value) -> Result<()> {
    eprintln!("{}", serde_json::to_string(value)?);
    Ok(())
}

fn print_resolved(resolved: &Resolved) {
    match resolved.theme() {
        Some(theme) => {
            println!(
                "{} ({}): {}",
                theme.name(),
                theme.id(),
                theme.source().label()
            );
            if let Some(variant) = theme.variant() {
                println!("Variant: {variant}");
            }
        }
        None => println!("No theme; the desktop uses the default theme."),
    }
    if let Some(fallback) = resolved.fallback() {
        println!("Fallback: {}", fallback.message());
    }
    for problem in resolved.diagnostics() {
        eprintln!("{problem}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_files_supply_named_bucket_roots() {
        let temp = tempfile::tempdir().expect("fixture");
        let registrations = temp.path().join("buckets/registrations");
        fs::create_dir_all(&registrations).expect("registrations");
        fs::write(
            registrations.join("community.json"),
            br#"{"path":"C:\\bucket"}"#,
        )
        .expect("registration");
        fs::write(registrations.join("main.json"), br#"{"path":"C:\\main"}"#)
            .expect("main registration");
        let roots = registered_roots(temp.path());
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].bucket_id(), "community");
        assert_eq!(roots[0].root(), Path::new("C:\\bucket"));
    }
}
