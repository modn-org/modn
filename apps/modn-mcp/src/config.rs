use anyhow::{bail, Context, Result};
use clap::Parser;
use std::path::{Component, Path, PathBuf};

#[derive(Parser)]
#[command(about = "Scoped local stdio MCP companion for Modn")]
pub struct Options {
    #[arg(long)]
    backend: PathBuf,
    #[arg(long)]
    state_dir: PathBuf,
    #[arg(long = "allow-root", required = true)]
    roots: Vec<PathBuf>,
    #[arg(long)]
    pub allow_writes: bool,
    #[arg(long, default_value = "30", value_parser = clap::value_parser!(u64).range(1..=300))]
    pub timeout_seconds: u64,
}

pub struct Config {
    pub backend: PathBuf,
    pub state: PathBuf,
    pub roots: Vec<PathBuf>,
    pub allow_writes: bool,
    pub timeout_seconds: u64,
}

impl Config {
    pub fn new(options: Options) -> Result<Self> {
        let backend = options
            .backend
            .canonicalize()
            .context("backend must exist")?;
        if !backend.is_file() {
            bail!("backend must be a file");
        }
        let state = options
            .state_dir
            .canonicalize()
            .context("state must exist")?;
        if !state.is_dir() || state.file_name().is_none_or(|name| name != "modn") {
            bail!("state directory must be an existing directory named modn");
        }
        let roots: Vec<_> = options
            .roots
            .iter()
            .map(|root| root.canonicalize())
            .collect::<std::io::Result<_>>()?;
        if roots.len() > 8 || roots.iter().any(|root| !root.is_dir()) {
            bail!("provide one to eight directory roots");
        }
        let config = Self {
            backend,
            state,
            roots,
            allow_writes: options.allow_writes,
            timeout_seconds: options.timeout_seconds,
        };
        config.check_state()?;
        Ok(config)
    }

    pub fn check_state(&self) -> Result<()> {
        let metadata = std::fs::symlink_metadata(&self.state)?;
        if redirected(&metadata) || !metadata.is_dir() || self.state.canonicalize()? != self.state {
            bail!("manager directory is redirected or invalid");
        }
        let mut pending = vec![self.state.clone()];
        let mut count = 0;
        while let Some(directory) = pending.pop() {
            for entry in std::fs::read_dir(directory)? {
                count += 1;
                if count > 20_000 {
                    bail!("manager tree exceeds entry limit");
                }
                let path = entry?.path();
                let metadata = std::fs::symlink_metadata(&path)?;
                if redirected(&metadata) || !path.canonicalize()?.starts_with(&self.state) {
                    bail!("manager tree contains a redirected path");
                }
                if metadata.is_dir() {
                    pending.push(path);
                }
            }
        }
        for relative in ["buckets/registrations", "game-modules"] {
            let directory = self.state.join(relative);
            if directory.exists() && std::fs::read_dir(directory)?.next().is_some() {
                bail!("MVP requires a dedicated state without community bucket or module registrations");
            }
        }
        Ok(())
    }

    pub fn input_path(&self, text: &str, directory: bool) -> Result<PathBuf> {
        if text.len() > 4096 {
            bail!("input path exceeds limit");
        }
        let path = Path::new(text);
        if !path.is_absolute()
            || path
                .components()
                .any(|part| matches!(part, Component::ParentDir))
        {
            bail!("input path must be absolute without parent traversal");
        }
        let canonical = path.canonicalize().context("input path must exist")?;
        if !self.roots.iter().any(|root| canonical.starts_with(root)) {
            bail!("input path is outside allowed roots");
        }
        let mut ancestor = Some(path);
        while let Some(path) = ancestor {
            if redirected(&std::fs::symlink_metadata(path)?) {
                bail!("input path must use canonical components without links");
            }
            ancestor = path.parent();
        }
        if canonical.is_dir() != directory || (!directory && !canonical.is_file()) {
            bail!("input path has the wrong file type");
        }
        if directory && (self.state.starts_with(&canonical) || canonical.starts_with(&self.state)) {
            bail!("manager state and game directory must be separate");
        }
        Ok(canonical)
    }
}

fn redirected(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

pub fn identifier(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 200
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_./".contains(&c))
        || value.starts_with('-')
        || value.contains("..")
        || value.starts_with('/')
    {
        bail!("invalid identifier");
    }
    Ok(())
}
