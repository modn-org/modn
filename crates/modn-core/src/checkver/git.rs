//! Git branch resolution through the existing Git transport.

use super::endpoints::{github_remote_url, test_endpoints_enabled};
use crate::git::{self, GitEnvironment};
use anyhow::{bail, Context, Result};

fn valid_commit(commit: &str) -> bool {
    commit.len() == 40
        && commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Resolve the head commit of one GitHub branch.
///
/// The call uses the same hardened Git transport as a pinned Git source. A
/// test binary may point the GitHub host at a local bare repository.
///
/// # Errors
///
/// Returns an error for a malformed repository or branch, a missing branch,
/// an ambiguous answer, or a Git failure.
pub fn git_branch_commit(repository: &str, branch: &str) -> Result<String> {
    let url = github_remote_url(repository)?;
    let directory = tempfile::Builder::new()
        .prefix("modn-checkver-git-")
        .tempdir()
        .context("cannot create a temporary Git directory")?;
    let hooks = git::empty_hooks()?;
    let environment = GitEnvironment {
        directory: directory.path(),
        hooks: hooks.path(),
        allow_file: test_endpoints_enabled(),
    };
    let reference = format!("refs/heads/{branch}");
    let output = git::run(&environment, &["ls-remote", "--", &url, &reference])
        .with_context(|| format!("cannot read branch {branch} of {url}"))?;
    let mut lines = output.lines().filter(|line| !line.trim().is_empty());
    let line = lines
        .next()
        .with_context(|| format!("Git branch not found: {url} {reference}"))?;
    if lines.next().is_some() {
        bail!("Git returned several commits for {url} {reference}");
    }
    let (commit, name) = line
        .split_once('\t')
        .with_context(|| format!("unexpected Git ls-remote output: {line}"))?;
    if name != reference {
        bail!("Git resolved {name} instead of {reference}");
    }
    if !valid_commit(commit) {
        bail!("Git returned an invalid commit for {url} {reference}");
    }
    Ok(commit.to_owned())
}

/// The deterministic package version of a Git commit.
///
/// The value is `git.<40 lowercase hex characters>`: 44 ASCII characters,
/// which passes the package manifest version rule and changes with the commit.
///
/// # Errors
///
/// Returns an error when `commit` is not exactly 40 lowercase hex characters.
pub fn git_source_version(commit: &str) -> Result<String> {
    if !valid_commit(commit) {
        bail!("Git checkver commit must be exactly 40 lowercase hex characters");
    }
    Ok(format!("git.{commit}"))
}

#[cfg(test)]
mod tests {
    use super::git_source_version;

    #[test]
    fn checkver_git_version_is_deterministic_and_valid() {
        let commit = "a".repeat(40);
        let version = git_source_version(&commit).expect("version");
        assert_eq!(version, format!("git.{commit}"));
        assert_eq!(git_source_version(&commit).expect("version"), version);
        assert!(version.len() <= 64);
        assert!(version
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b'/' && byte != b'\\'));
    }

    #[test]
    fn checkver_git_version_refuses_a_short_or_uppercase_commit() {
        assert!(git_source_version("abc").is_err());
        assert!(git_source_version(&"A".repeat(40)).is_err());
    }
}
