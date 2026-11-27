//! `modn catalog init`: scaffold a contract v2 bucket repository.

use anyhow::{bail, Context, Result};
use modn_core::buckets::validate_bucket_metadata;
use serde::Serialize;
use serde_json::json;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const BUCKET_JSON: &str = "bucket.json";
const MODS: &str = "mods";
const PROVENANCE: &str = "PROVENANCE.md";
const GITIGNORE: &str = ".gitignore";
const PACKAGE_TEMPLATE: &str = "templates/package.example.json";
const VALIDATE_WORKFLOW: &str = ".github/workflows/validate.yml";
const AUTOUPDATE_WORKFLOW: &str = ".github/workflows/autoupdate.yml";

const PROVENANCE_TEMPLATE: &str = include_str!("../../templates/bucket/PROVENANCE.md");
const PACKAGE_EXAMPLE: &str = include_str!("../../templates/bucket/package.example.json");
const GITIGNORE_TEMPLATE: &str = include_str!("../../templates/bucket/.gitignore");
const VALIDATE_TEMPLATE: &str = include_str!("../../templates/bucket/validate.yml");
const AUTOUPDATE_TEMPLATE: &str = include_str!("../../templates/bucket/autoupdate.yml");
const MODN_VERSION: &str = env!("CARGO_PKG_VERSION");

/// `bucket.json` in the field order that the repository uses.
#[derive(Serialize)]
struct BucketMetadata<'a> {
    schema_version: u32,
    name: &'a str,
    title: &'a str,
}

/// The files and directories that one scaffold writes, relative to its root.
struct Scaffold {
    bucket_json: String,
    validate_workflow: String,
    autoupdate_workflow: String,
}

impl Scaffold {
    fn new(name: &str, title: &str) -> Result<Self> {
        let metadata = BucketMetadata {
            schema_version: 2,
            name,
            title,
        };
        Ok(Self {
            bucket_json: format!(
                "{}\n",
                serde_json::to_string_pretty(&metadata)
                    .context("cannot encode the scaffolded bucket.json")?
            ),
            validate_workflow: VALIDATE_TEMPLATE.replace("{{VERSION}}", MODN_VERSION),
            autoupdate_workflow: AUTOUPDATE_TEMPLATE.replace("{{VERSION}}", MODN_VERSION),
        })
    }

    fn files(&self) -> [(&'static str, &[u8]); 6] {
        [
            (BUCKET_JSON, self.bucket_json.as_bytes()),
            (PROVENANCE, PROVENANCE_TEMPLATE.as_bytes()),
            (PACKAGE_TEMPLATE, PACKAGE_EXAMPLE.as_bytes()),
            (GITIGNORE, GITIGNORE_TEMPLATE.as_bytes()),
            (VALIDATE_WORKFLOW, self.validate_workflow.as_bytes()),
            (AUTOUPDATE_WORKFLOW, self.autoupdate_workflow.as_bytes()),
        ]
    }
}

pub(super) fn run(dir: &Path, name: &str, title: &str, json_output: bool) -> Result<()> {
    validate_bucket_metadata(name, title)?;
    check_target(dir)?;
    let scaffold = Scaffold::new(name, title)?;
    let mut created = Created::default();
    let written = write_all(dir, &scaffold, &mut created);
    if let Err(error) = written {
        created.rollback();
        return Err(error);
    }
    if json_output {
        println!(
            "{}",
            json!({
                "directory": dir,
                "name": name,
                "title": title,
                "files": scaffold.files().iter()
                    .map(|(relative, _)| *relative)
                    .collect::<Vec<_>>(),
            })
        );
    } else {
        println!("Created catalog bucket {name} in {}", dir.display());
        println!("  {MODS}/");
        for (relative, _) in scaffold.files() {
            println!("  {relative}");
        }
        println!();
        println!("Next steps:");
        println!("  1. Run `modn catalog keygen --out <key directory outside this catalog>`.");
        println!("  2. Copy templates/package.example.json to mods/<game-id>/<package-id>.json and fill in the placeholders.");
        println!("  3. Run `modn catalog sign . --key <key directory>/catalog.key`.");
        println!("  4. Run `modn catalog verify . --key catalog.pub`.");
        println!("  5. Push the repository and enable the validate and autoupdate workflows.");
    }
    Ok(())
}

/// Refuse a target that exists with content, or that is not a real directory.
fn check_target(dir: &Path) -> Result<()> {
    match fs::symlink_metadata(dir) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                bail!(
                    "refusing to initialize a linked path: {}; choose a real directory",
                    dir.display()
                );
            }
            if !metadata.is_dir() {
                bail!("target exists and is not a directory: {}", dir.display());
            }
            let mut entries = fs::read_dir(dir)
                .with_context(|| format!("cannot read target directory: {}", dir.display()))?;
            if entries.next().is_some() {
                bail!(
                    "refusing to initialize a non-empty directory: {}; choose a new directory or empty it first",
                    dir.display()
                );
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("cannot read target directory: {}", dir.display()))
        }
    }
    Ok(())
}

fn write_all(root: &Path, scaffold: &Scaffold, created: &mut Created) -> Result<()> {
    created.ensure_dir(root)?;
    created.ensure_dir(&root.join(MODS))?;
    for (relative, bytes) in scaffold.files() {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            created.ensure_dir(parent)?;
        }
        fs::write(&path, bytes)
            .with_context(|| format!("cannot write scaffolded file: {}", path.display()))?;
        created.files.push(path);
    }
    Ok(())
}

#[derive(Default)]
struct Created {
    files: Vec<PathBuf>,
    directories: Vec<PathBuf>,
}

impl Created {
    /// Create one directory and remember each directory that did not exist.
    fn ensure_dir(&mut self, path: &Path) -> Result<()> {
        if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir()) {
            return Ok(());
        }
        let mut missing = Vec::new();
        let mut current = Some(path);
        while let Some(directory) = current {
            if fs::symlink_metadata(directory).is_ok() {
                break;
            }
            missing.push(directory.to_owned());
            current = directory.parent();
        }
        fs::create_dir_all(path)
            .with_context(|| format!("cannot create scaffold directory: {}", path.display()))?;
        missing.reverse();
        self.directories.extend(missing);
        Ok(())
    }

    fn rollback(&self) {
        for file in self.files.iter().rev() {
            let _ = fs::remove_file(file);
        }
        for directory in self.directories.iter().rev() {
            let _ = fs::remove_dir(directory);
        }
    }
}
