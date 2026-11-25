use super::support::Fixture;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.is_file() {
        assert!(Instant::now() < deadline, "{path:?} never appeared");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn game_launch_returns_before_the_started_game_exits() {
    let f = Fixture::new();
    let args = f.registration_args("game-launch");
    let backend = f
        .backend
        .clone()
        .with_inactivity_timeout(Duration::from_secs(5));
    let result = backend
        .call_with_inactivity_timeout(&args)
        .expect("launch result");
    assert_eq!(result["game_id"], "fixture");
    let state = &f.backend.state;
    wait_for_file(&state.join("game-started"));
    assert!(
        !state.join("game-exited").exists(),
        "the desktop must not wait for the started game to exit"
    );
    fs::write(state.join("game-release"), b"yes").expect("release the fixture game");
    wait_for_file(&state.join("game-exited"));
}
