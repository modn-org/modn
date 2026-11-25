use super::*;

fn test_backend() -> Backend {
    Backend {
        binary: PathBuf::from(r"C:\modn-test\missing-backend.exe"),
        state: PathBuf::from(r"C:\modn-test\state"),
        data: None,
        modules: None,
        evidence: None,
        inactivity_timeout: None,
    }
}

fn test_game() -> Game {
    Game {
        game_id: "fixture".into(),
        path: PathBuf::from(r"C:\modn-test\game"),
    }
}

#[test]
fn install_refuses_a_blocked_plan_before_reading_the_archive() {
    let review = Review {
        game: test_game(),
        archive: PathBuf::from(r"C:\modn-test\missing.zip"),
        digest: Vec::new(),
        plan: serde_json::json!({"status": "blocked"}),
    };
    assert!(test_backend().install(&review).is_err());
}

#[test]
fn games_and_uninstall_fail_closed_when_the_backend_is_missing() {
    let backend = test_backend();
    assert!(backend.games().is_err());
    assert!(backend.uninstall(&test_game(), "package").is_err());
}

#[test]
fn inactivity_timeout_is_recorded() {
    let backend = test_backend().with_inactivity_timeout(Duration::from_millis(5));
    assert_eq!(backend.inactivity_timeout, Some(Duration::from_millis(5)));
}

fn args(list: &[&str]) -> Vec<OsString> {
    list.iter().map(|value| OsString::from(*value)).collect()
}

fn launch_args() -> Vec<OsString> {
    let state = std::env::temp_dir().join(format!(
        "modn-desktop-args-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    vec![
        "--backend".into(),
        std::env::current_exe()
            .expect("the test executable")
            .into_os_string(),
        "--state-dir".into(),
        state.into_os_string(),
    ]
}

#[test]
fn the_default_theme_flag_is_found_anywhere_in_the_arguments() {
    assert!(has_default_theme_flag(&args(&["--default-theme"])));
    assert!(has_default_theme_flag(&args(&[
        "--backend",
        "backend.exe",
        "--default-theme",
        "--state-dir",
        "state"
    ])));
    assert!(!has_default_theme_flag(&args(&[
        "--backend",
        "backend.exe"
    ])));
    assert!(!has_default_theme_flag(&[]));
}

#[test]
fn launch_arguments_without_the_flag_build_the_backend() {
    let backend = Backend::parse_args(launch_args()).expect("valid launch arguments");
    assert!(backend.binary.is_file());
    assert!(backend.state.is_absolute());
    assert!(backend.data.is_none());
    assert!(backend.modules.is_none());
    assert!(backend.evidence.is_none());
}

#[test]
fn launch_arguments_with_the_flag_build_the_same_backend() {
    let plain = launch_args();
    let mut flagged = plain.clone();
    flagged.push(DEFAULT_THEME_FLAG.into());
    let plain = Backend::parse_args(plain).expect("valid launch arguments");
    let flagged = Backend::parse_args(flagged).expect("the flag is skipped");
    assert_eq!(flagged.binary, plain.binary);
    assert_eq!(flagged.state, plain.state);
}

#[test]
fn the_flag_between_options_does_not_consume_the_next_path() {
    let mut list = launch_args();
    list.insert(2, DEFAULT_THEME_FLAG.into());
    let backend = Backend::parse_args(list).expect("valid launch arguments");
    assert!(backend.binary.is_file());
    assert!(backend.state.is_absolute());
}

#[test]
fn the_flag_takes_no_value_so_a_following_argument_is_unknown() {
    let state = std::env::temp_dir().join("modn-desktop-args-value");
    let list = vec![
        OsString::from("--backend"),
        std::env::current_exe()
            .expect("the test executable")
            .into_os_string(),
        OsString::from(DEFAULT_THEME_FLAG),
        OsString::from("extra"),
        state.clone().into_os_string(),
        OsString::from("--state-dir"),
        state.into_os_string(),
    ];
    let error = Backend::parse_args(list).expect_err("the flag takes no value");
    assert!(format!("{error:#}").contains("Unknown option"));
}

#[test]
fn only_complete_operations_in_unblocked_metadata_plans_enable_install() {
    let mut plan = serde_json::json!({"status":"reviewable", "metadata_only":true, "blockers":[], "operations":[{"source":"plugins/demo.plugin", "destination":"addons/demo.plugin"}]});
    assert!(reviewable(&plan));
    for operations in [
        serde_json::json!([{}]),
        serde_json::json!([null]),
        serde_json::json!([{"source":"plugins/demo.plugin"}]),
        serde_json::json!([{"destination":"addons/demo.plugin"}]),
        serde_json::json!([{"source":1, "destination":"addons/demo.plugin"}]),
        serde_json::json!([{"source":"plugins/demo.plugin", "destination":null}]),
        serde_json::json!([{"source":"", "destination":"addons/demo.plugin"}]),
        serde_json::json!([{"source":"plugins/demo.plugin", "destination":" "}]),
        serde_json::json!([{"source":"plugins/demo.plugin", "destination":"addons/demo.plugin"}, {}]),
    ] {
        let mut malformed = plan.clone();
        malformed["operations"] = operations;
        assert!(!reviewable(&malformed), "malformed operations: {malformed}");
    }
    for blocked in [
        serde_json::json!([{"message":"approval required"}]),
        Value::Null,
    ] {
        plan["blockers"] = blocked;
        assert!(!reviewable(&plan));
    }
    assert!(!reviewable(&serde_json::json!({})));
}
