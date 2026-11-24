use crate::themes::{Rule, ThemeError, ThemeProblem};
use std::error::Error;
use std::io;
use std::path::PathBuf;

fn problem() -> ThemeProblem {
    ThemeProblem::new(Rule::ManifestInvalid, "theme.json is missing")
}

#[test]
fn invalid_reports_the_first_problem_and_keeps_every_problem() {
    let error = ThemeError::Invalid(vec![
        problem(),
        ThemeProblem::new(Rule::DigestMismatch, "the digest changed"),
    ]);
    assert!(error.to_string().contains("manifest-invalid"));
    match &error {
        ThemeError::Invalid(problems) => {
            assert_eq!(problems.len(), 2);
            assert_eq!(problems[1].rule, Rule::DigestMismatch);
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    assert!(error.source().is_none());
}

#[test]
fn a_problem_converts_into_an_invalid_error() {
    let error: ThemeError = problem().into();
    match error {
        ThemeError::Invalid(problems) => {
            assert_eq!(problems.len(), 1);
            assert_eq!(problems[0].rule, Rule::ManifestInvalid);
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn io_keeps_the_source_and_displays_the_context() {
    let error = ThemeError::Io {
        context: "cannot copy the theme".to_owned(),
        source: io::Error::new(io::ErrorKind::PermissionDenied, "denied"),
    };
    assert_eq!(error.to_string(), "cannot copy the theme: denied");
    assert_eq!(
        error
            .source()
            .expect("io errors expose a source")
            .to_string(),
        "denied"
    );
}

#[test]
fn corrupt_names_the_path_and_reason() {
    let error = ThemeError::Corrupt {
        path: PathBuf::from("registry/game/theme/registration.json"),
        reason: "not valid JSON".to_owned(),
    };
    let text = error.to_string();
    assert!(text.contains("registration.json"), "{text}");
    assert!(text.contains("not valid JSON"), "{text}");
    assert!(error.source().is_none());
}

#[test]
fn not_registered_ambiguous_and_invalid_id_explain_the_failure() {
    let missing = ThemeError::NotRegistered {
        id: "night-city".to_owned(),
    };
    assert_eq!(
        missing.to_string(),
        "no theme with id \"night-city\" is registered"
    );
    let ambiguous = ThemeError::Ambiguous {
        id: "night-city".to_owned(),
        games: vec!["game-a".to_owned(), "game-b".to_owned()],
    };
    let text = ambiguous.to_string();
    assert!(text.contains("game-a, game-b"), "{text}");
    assert!(text.contains("pass the game"), "{text}");
    let invalid = ThemeError::InvalidId("Night-City".to_owned());
    assert!(invalid.to_string().contains("Night-City"));
}
