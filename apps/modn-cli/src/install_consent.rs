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
    ask_approval(request.executables, input, output)
}

/// What the operator approves for a mod installed with its missing dependencies.
pub(super) struct StackRequest<'a> {
    pub(super) game: &'a Path,
    /// `reference version` of each package, in install order.
    pub(super) packages: &'a [String],
    pub(super) executables: &'a [String],
}

/// Asks once for the whole stack, with the same answers as [`confirm`].
pub(super) fn confirm_stack(
    request: &StackRequest<'_>,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> io::Result<bool> {
    writeln!(
        output,
        "Modn will install {} package(s) into {}, in this order:",
        request.packages.len(),
        request.game.display()
    )?;
    for package in request.packages {
        writeln!(output, "  {package}")?;
    }
    writeln!(
        output,
        "The game policy plans each package after the packages before it are installed."
    )?;
    writeln!(
        output,
        "If any package fails, Modn uninstalls the packages this command installed."
    )?;
    ask_approval(request.executables, input, output)
}

fn ask_approval(
    executables: &[String],
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> io::Result<bool> {
    if !executables.is_empty() {
        writeln!(
            output,
            "It installs {} executable(s) declared by its catalog manifest:",
            executables.len()
        )?;
        for path in executables {
            writeln!(output, "  {path}")?;
        }
        writeln!(
            output,
            "Approving does not certify these executables as trustworthy."
        )?;
    }
    write!(output, "Install? [y/N]: ")?;
    read_yes(input, output)
}

/// Asks before downloads that must happen ahead of the install prompt.
pub(super) fn confirm_download(
    downloads: &[String],
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> io::Result<bool> {
    writeln!(output, "Modn will download into its cache:")?;
    for download in downloads {
        writeln!(output, "  {download}")?;
    }
    write!(output, "Download? [y/N]: ")?;
    read_yes(input, output)
}

fn read_yes(input: &mut impl BufRead, output: &mut impl Write) -> io::Result<bool> {
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

    #[test]
    fn stack_prompt_lists_packages_in_order_and_needs_an_explicit_yes() {
        let packages = [
            "main/cyberpunk-2077/red4ext 1.31.0".to_owned(),
            "main/cyberpunk-2077/cet 1.38.1".to_owned(),
        ];
        let request = StackRequest {
            game: Path::new("game"),
            packages: &packages,
            executables: &[],
        };
        let mut output = Vec::new();
        let approved =
            confirm_stack(&request, &mut Cursor::new("\n"), &mut output).expect("prompt");
        let text = String::from_utf8(output).expect("UTF-8");
        assert!(!approved);
        assert!(text.contains("install 2 package(s)"), "{text}");
        let red4ext = text.find("red4ext 1.31.0").expect("red4ext");
        assert!(red4ext < text.find("cet 1.38.1").expect("cet"), "{text}");
        assert!(text.contains("uninstalls the packages"), "{text}");
    }

    #[test]
    fn download_prompt_names_each_download_and_needs_an_explicit_yes() {
        let downloads =
            ["main/cyberpunk-2077/red4ext 1.31.0 from https://example.invalid/r.zip".to_owned()];
        for (answer, expected) in [("y", true), ("", false), ("no", false)] {
            let mut output = Vec::new();
            let approved = confirm_download(&downloads, &mut Cursor::new(answer), &mut output)
                .expect("prompt");
            let text = String::from_utf8(output).expect("UTF-8");
            assert_eq!(approved, expected, "{answer:?}");
            assert!(
                text.contains("red4ext 1.31.0 from https://example.invalid/r.zip"),
                "{text}"
            );
            assert!(text.ends_with("Download? [y/N]: "), "{text}");
        }
    }
}
