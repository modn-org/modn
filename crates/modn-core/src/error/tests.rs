use super::*;
use anyhow::Context;
use std::collections::BTreeSet;

#[test]
fn every_code_is_unique_and_screaming_snake_case() {
    let names: BTreeSet<&str> = ErrorCode::ALL.iter().map(|code| code.as_str()).collect();
    assert_eq!(names.len(), ErrorCode::ALL.len());
    for name in names {
        assert!(
            name.bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte == b'_'),
            "{name}"
        );
        assert!(!name.starts_with('_') && !name.ends_with('_'), "{name}");
    }
}

#[test]
fn every_code_is_documented_in_the_command_reference() {
    let reference = include_str!("../../../../docs/cli.md");
    for code in ErrorCode::ALL {
        assert!(
            reference.contains(&format!("`{}`", code.as_str())),
            "docs/cli.md does not document {}",
            code.as_str()
        );
    }
}

#[test]
fn codes_serialize_as_their_names() {
    assert_eq!(
        serde_json::to_value(ErrorCode::PendingTransaction).expect("code JSON"),
        "PENDING_TRANSACTION"
    );
}

#[test]
fn launch_and_detect_codes_serialize_as_their_names() {
    for code in [
        ErrorCode::DetectFolderUnconfirmed,
        ErrorCode::DetectFolderUnreadable,
        ErrorCode::DetectModuleFailed,
        ErrorCode::DetectModuleUnreadable,
        ErrorCode::LaunchDriftCheckFailed,
        ErrorCode::LaunchGameMismatch,
        ErrorCode::LaunchInstallationInvalid,
        ErrorCode::LaunchInstallationUnregistered,
        ErrorCode::LaunchManagedFilesChanged,
        ErrorCode::LaunchRecoveryPending,
        ErrorCode::LaunchStartFailed,
        ErrorCode::LaunchTargetUnavailable,
        ErrorCode::LaunchTargetUnsafe,
        ErrorCode::ModuleLaunchTargetInvalid,
        ErrorCode::PlayFailed,
    ] {
        assert!(ErrorCode::ALL.contains(&code), "{}", code.as_str());
        assert_eq!(
            serde_json::to_value(code).expect("code JSON"),
            code.as_str()
        );
    }
}

#[test]
fn coded_errors_keep_their_message_and_code_through_context() {
    let error = Err::<(), _>(coded(ErrorCode::PlanBlocked, "plan has blockers"))
        .context("outer step")
        .expect_err("coded error");
    assert_eq!(format!("{error:#}"), "outer step: plan has blockers");
    assert_eq!(code_of(&error), Some(ErrorCode::PlanBlocked));
}

#[test]
fn cancellation_wins_over_other_codes() {
    let error = anyhow::Error::new(Cancelled).context(CodedError::new(
        ErrorCode::DownloadFailed,
        "download failed",
    ));
    assert_eq!(code_of(&error), Some(ErrorCode::Cancelled));
}

#[test]
fn with_code_keeps_an_existing_specific_code() {
    let inner = coded(ErrorCode::DependencyMissing, "needs cet");
    let error = with_code(inner, ErrorCode::InstallFailed);
    assert_eq!(code_of(&error), Some(ErrorCode::DependencyMissing));
}

#[test]
fn with_code_tags_an_uncoded_error_and_keeps_its_text() {
    let inner = anyhow::anyhow!("root cause").context("while staging");
    let error = with_code(inner, ErrorCode::DownloadFailed);
    assert_eq!(code_of(&error), Some(ErrorCode::DownloadFailed));
    assert_eq!(format!("{error:#}"), "while staging: root cause");
}

#[test]
fn known_error_types_have_codes_below_context() {
    let io = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
    let error = anyhow::Error::new(io).context("first").context("second");
    assert_eq!(code_of(&error), Some(ErrorCode::IoError));
    let sqlite = anyhow::Error::new(rusqlite::Error::InvalidQuery).context("query");
    assert_eq!(code_of(&sqlite), Some(ErrorCode::StateError));
}

#[test]
fn plain_messages_have_no_specific_code() {
    assert_eq!(code_of(&anyhow::anyhow!("something went wrong")), None);
}
