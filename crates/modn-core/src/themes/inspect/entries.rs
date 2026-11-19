//! Top-level entry checks for one theme directory.

use super::super::manifest::MANIFEST_FILE;
use super::super::problem::{Rule, ThemeProblem};
use super::{ASSETS_DIR, CSS_FILE};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

/// Top-level files that a theme may contain without affecting it.
const IGNORED_FILES: [&str; 3] = ["README.md", "LICENSE", "NOTICE.md"];

/// What the top-level directory listing contained.
#[derive(Debug, Default)]
pub(super) struct TopLevel {
    pub(super) seen: BTreeSet<String>,
    pub(super) files: BTreeMap<String, u64>,
    pub(super) assets: bool,
}

/// Check one top-level entry against the allowed theme folder layout.
pub(super) fn inspect_entry(
    entry: &fs::DirEntry,
    variants: &[String],
    top: &mut TopLevel,
    problems: &mut Vec<ThemeProblem>,
) {
    let name = match entry.file_name().into_string() {
        Ok(name) => name,
        Err(_) => {
            problems.push(
                ThemeProblem::new(
                    Rule::UnexpectedFile,
                    "a top-level entry name is not valid UTF-8",
                )
                .with_path("."),
            );
            return;
        }
    };
    top.seen.insert(name.clone());
    let file_type = match entry.file_type() {
        Ok(file_type) => file_type,
        Err(error) => {
            problems.push(
                ThemeProblem::new(
                    Rule::UnexpectedFile,
                    format!("cannot inspect \"{name}\": {error}"),
                )
                .with_path(&name),
            );
            return;
        }
    };
    if file_type.is_symlink() {
        problems.push(
            ThemeProblem::new(
                Rule::UnexpectedFile,
                format!("link \"{name}\" is refused; links are not allowed"),
            )
            .with_path(&name),
        );
        return;
    }
    if name == ASSETS_DIR {
        if file_type.is_dir() {
            top.assets = true;
        } else {
            problems.push(
                ThemeProblem::new(
                    Rule::UnexpectedFile,
                    format!("\"{ASSETS_DIR}\" must be a directory"),
                )
                .with_path(&name),
            );
        }
        return;
    }
    if !file_type.is_file() {
        problems.push(
            ThemeProblem::new(
                Rule::UnexpectedFile,
                format!("\"{name}\" must be a regular file"),
            )
            .with_path(&name),
        );
        return;
    }
    let size = match entry.metadata() {
        Ok(metadata) => metadata.len(),
        Err(error) => {
            problems.push(
                ThemeProblem::new(
                    Rule::UnexpectedFile,
                    format!("cannot inspect \"{name}\": {error}"),
                )
                .with_path(&name),
            );
            return;
        }
    };
    top.files.insert(name.clone(), size);
    if allowed_file(&name, variants) {
        return;
    }
    problems.push(
        ThemeProblem::new(
            Rule::UnexpectedFile,
            format!("\"{name}\" is not allowed in a theme directory"),
        )
        .with_path(&name),
    );
}

fn allowed_file(name: &str, variants: &[String]) -> bool {
    name == MANIFEST_FILE
        || name == CSS_FILE
        || IGNORED_FILES.contains(&name)
        || variant_of_file(name)
            .is_some_and(|variant| variants.iter().any(|value| value == variant))
}

/// Return the variant id in a `theme.<variant>.css` file name.
pub(super) fn variant_of_file(name: &str) -> Option<&str> {
    let variant = name.strip_prefix("theme.")?.strip_suffix(".css")?;
    if variant.is_empty() {
        None
    } else {
        Some(variant)
    }
}
