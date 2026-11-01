//! Operator confirmation for catalog installs and their declared executables.
//!
//! A bucket manifest may *declare* tool executables, but only the operator
//! approves them: interactively at this prompt, or with an explicit
//! `--allow-executable` flag naming each path. `--yes` alone never approves a
//! declared executable.
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::Path;

/// Whether this invocation can ask the operator a question.
pub(super) fn can_prompt(json: bool) -> bool {
    !json && io::stdin().is_terminal() && io::stderr().is_terminal()
}

/// Declared executables among a plan's destinations that no flag approved.
pub(super) fn pending_executables(
    declared: &[String],
    explicit: &[String],
    destinations: &[&str],
) -> Vec<String> {
    declared
        .iter()
        .filter(|path| !explicit.contains(path))
        .filter(|path| destinations.contains(&path.as_str()))
        .cloned()
        .collect()
}

/// What the operator is asked to approve in one confirmation.
pub(super) struct Request<'a> {
    pub(super) reference: &'a str,
    pub(super) game: &'a Path,
    pub(super) files: usize,
    pub(super) executables: &'a [String],
}

/// Asks once; only `y` or `yes` approves. Anything else, including EOF,
/// declines.
pub(super) fn confirm(
    request: &Request<'_>,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> io::Result<bool> {
    writeln!(
        output,
        "{} will write {} file(s) into {}.",
        request.reference,
        request.files,
        request.game.display()
    )?;
    if !request.executables.is_empty() {
        writeln!(
            output,
            "It installs {} executable(s) declared by its catalog manifest:",
            request.executables.len()
        )?;
        for path in request.executables {
            writeln!(output, "  {path}")?;
        }
        writeln!(
            output,
            "Approving does not certify these executables as trustworthy."
        )?;
    }
    write!(output, "Install? [y/N]: ")?;
    output.flush()?;
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn ask(executables: &[String], answer: &str) -> (bool, String) {
        let request = Request {
            reference: "main/cyberpunk-2077/redscript",
            game: Path::new("game"),
            files: 4,
            executables,
        };
        let mut output = Vec::new();
        let approved = confirm(&request, &mut Cursor::new(answer), &mut output).expect("prompt");
        (approved, String::from_utf8(output).expect("UTF-8"))
    }

    #[test]
    fn only_an_explicit_yes_approves() {
        let compiler = ["engine/tools/scc.exe".to_owned()];
        for (answer, expected) in [
            ("y\n", true),
            ("YES\n", true),
            (" yes \r\n", true),
            ("\n", false),
            ("", false),
            ("n\n", false),
            ("yep\n", false),
        ] {
            assert_eq!(ask(&compiler, answer).0, expected, "{answer:?}");
        }
    }

    #[test]
    fn only_declared_unapproved_installed_executables_are_pending() {
        let owned = |paths: &[&str]| paths.iter().map(|&p| p.to_owned()).collect::<Vec<_>>();
        let declared = owned(&[
            "engine/tools/a.exe",
            "engine/tools/b.exe",
            "engine/tools/absent.exe",
        ]);
        let explicit = owned(&["engine/tools/a.exe", "engine/tools/undeclared.exe"]);
        let destinations = [
            "engine/tools/a.exe",
            "engine/tools/b.exe",
            "engine/tools/undeclared.exe",
            "engine/tools/other.exe",
            "r6/scripts/mod.reds",
        ];
        // a: already approved by flag; absent: not installed; undeclared and
        // other: never approvable by the manifest.
        assert_eq!(
            pending_executables(&declared, &explicit, &destinations),
            ["engine/tools/b.exe"]
        );
        assert!(pending_executables(&[], &explicit, &destinations).is_empty());
    }

    #[test]
    fn prompt_names_every_declared_executable() {
        let paths = [
            "engine/tools/a.exe".to_owned(),
            "engine/tools/b.exe".to_owned(),
        ];
        let (_, text) = ask(&paths, "n\n");
        assert!(text.contains("2 executable(s)"), "{text}");
        assert!(text.contains("engine/tools/a.exe") && text.contains("engine/tools/b.exe"));
        let (_, plain) = ask(&[], "n\n");
        assert!(!plain.contains("executable"), "{plain}");
    }
}
