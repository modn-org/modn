#[expect(
    dead_code,
    reason = "The theme tests reuse the application backend without invoking its process-global argument parser"
)]
#[cfg(windows)]
#[path = "../src/backend.rs"]
mod backend;

#[expect(
    dead_code,
    reason = "The theme tests include every argument builder but pin only the select request"
)]
#[cfg(windows)]
#[path = "../src/app/commands/args.rs"]
mod args;

#[expect(
    dead_code,
    reason = "The theme tests include the command error, busy guard and blocking runner, not every command-layer item"
)]
#[cfg(windows)]
// The path names an existing directory, so rustfmt on Linux can resolve it.
#[path = "../src/app"]
mod app {
    pub(crate) mod blocking;
    pub(crate) mod error;
    pub(crate) mod state;
    pub(crate) mod theme_diagnostics;
}

#[cfg(windows)]
mod theme_commands {

    use super::backend;
    use crate::app;
    use crate::args;
    use serde_json::Value;
    use std::ffi::OsString;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture {
        state: PathBuf,
        backend: backend::Backend,
    }

    impl Fixture {
        fn new(binary: &str) -> Self {
            static SEQUENCE: AtomicU64 = AtomicU64::new(1);
            let state = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
                "modn-desktop-theme-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&state).expect("create the disposable state");
            fs::write(state.join("test-backend.marker"), b"disposable").expect("fixture marker");
            let backend = backend::Backend {
                binary: PathBuf::from(binary),
                state: state.clone(),
                data: None,
                modules: None,
                evidence: None,
                inactivity_timeout: None,
            };
            Self { state, backend }
        }

        fn call(&self, tail: &[&str]) -> Value {
            let busy = Arc::new(app::state::Busy::default());
            let backend = self.backend.clone();
            let args: Vec<OsString> = tail.iter().map(|arg| OsString::from(*arg)).collect();
            tauri::async_runtime::block_on(app::blocking::run_blocking(&busy, move || {
                backend.call(&args)
            }))
            .expect("the fixture answers")
        }

        fn recorded(&self) -> Vec<String> {
            serde_json::from_str(
                &fs::read_to_string(self.state.join("last-args.json"))
                    .expect("the fixture records its arguments"),
            )
            .expect("valid argument record")
        }

        fn expected(&self, tail: &[&str]) -> Vec<String> {
            let mut expected = vec![
                "--json".to_string(),
                "--state-dir".to_string(),
                self.state.to_string_lossy().into_owned(),
            ];
            expected.extend(tail.iter().map(|arg| (*arg).to_string()));
            expected
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.state).expect("remove only this fixture");
        }
    }

    fn fixture_binary() -> Option<&'static str> {
        if !cfg!(feature = "verification") {
            return None;
        }
        option_env!("CARGO_BIN_EXE_modn-desktop-test-backend")
    }

    #[test]
    fn theme_list_sends_the_game_selection() {
        let Some(binary) = fixture_binary() else {
            return;
        };
        let fixture = Fixture::new(binary);
        let tail = ["theme", "list", "--game", "fixture"];
        let answer = fixture.call(&tail);
        assert_eq!(fixture.recorded(), fixture.expected(&tail));
        assert_eq!(answer["themes"][0]["id"], "night-city");
        assert_eq!(answer["diagnostics"], serde_json::json!([]));
    }

    #[test]
    fn theme_resolve_names_an_existing_asset_directory() {
        let Some(binary) = fixture_binary() else {
            return;
        };
        let fixture = Fixture::new(binary);
        let tail = ["theme", "resolve", "--game", "fixture"];
        let answer = fixture.call(&tail);
        assert_eq!(fixture.recorded(), fixture.expected(&tail));
        let root = answer["theme"]["assets_dir"]
            .as_str()
            .expect("the fixture names its theme directory");
        assert!(PathBuf::from(root).is_absolute());
        assert!(PathBuf::from(root).is_dir());
        assert_eq!(answer["theme"]["css"], ".modn-app{}");
        assert_eq!(answer["fallback"], Value::Null);
    }

    #[test]
    fn theme_select_sends_the_pair_and_optional_variant() {
        let Some(binary) = fixture_binary() else {
            return;
        };
        let fixture = Fixture::new(binary);
        let tail = [
            "theme",
            "select",
            "fixture",
            "night-city",
            "--variant",
            "neon",
        ];
        let answer = fixture.call(&tail);
        assert_eq!(fixture.recorded(), fixture.expected(&tail));
        assert_eq!(answer["selected"]["game"], "fixture");
        assert_eq!(answer["selected"]["theme"], "night-city");
    }

    #[test]
    fn theme_select_none_sends_the_explicit_default() {
        let Some(binary) = fixture_binary() else {
            return;
        };
        let fixture = Fixture::new(binary);
        let tail = ["theme", "select", "fixture", "--none"];
        let built: Vec<String> = args::theme_select_none("fixture")
            .expect("a valid game id")
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(built, tail);
        fixture.call(&tail);
        assert_eq!(fixture.recorded(), fixture.expected(&tail));
    }

    #[test]
    fn theme_reset_reports_the_cleared_games() {
        let Some(binary) = fixture_binary() else {
            return;
        };
        let fixture = Fixture::new(binary);
        let tail = ["theme", "reset", "--game", "fixture"];
        let answer = fixture.call(&tail);
        assert_eq!(fixture.recorded(), fixture.expected(&tail));
        assert_eq!(answer["reset"][0], "fixture");
    }

    #[test]
    fn theme_register_sends_the_chosen_folder() {
        let Some(binary) = fixture_binary() else {
            return;
        };
        let fixture = Fixture::new(binary);
        let folder = fixture.state.join("import");
        fs::create_dir(&folder).expect("create the import folder");
        let folder = folder.to_string_lossy().into_owned();
        let tail = ["theme", "register", folder.as_str()];
        let answer = fixture.call(&tail);
        assert_eq!(fixture.recorded(), fixture.expected(&tail));
        assert_eq!(answer["registered"]["id"], "night-city");
        assert_eq!(answer["digest"], "blake3:fixture");
    }
}
