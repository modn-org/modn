---
files:
  - "apps/modn-cli/tests/**/*.rs"
  - "apps/modn-cli/src/*tests*.rs"
  - "apps/modn-cli/src/*tests*/**/*.rs"
  - "apps/modn-cli/src/**/tests.rs"
  - "crates/modn-core/tests/**/*.rs"
  - "crates/modn-core/src/**/tests.rs"
  - "crates/modn-core/src/**/tests/**/*.rs"
  - "crates/modn-core/src/**/*_tests.rs"
  - "crates/modn-core/src/**/*_tests/**/*.rs"
---

# Tests never touch the user's real Modn data or appdata

A test that runs the `modn` binary, or calls core code that resolves Modn's
data folders, must point every location that code can write at a temporary
folder. `--state-dir` alone is not enough:

- Core module sync, module import and the official `main` copy live under
  `data_dir::data_directory()`, which reads `LOCALAPPDATA` (then
  `XDG_STATE_HOME`, then `HOME`), even when the CLI gets `--state-dir`.
- Content-root resolution (`content_root::appdata_directory`) reads `APPDATA`,
  or the test-only `MODN_APPDATA`, for any game module that declares an
  `appdata` content root.
- `MODN_GAME_MODULES_DIR` overrides module resolution for every command in the
  environment; a test that does not set it must remove it so a developer's
  value does not leak in.

A test breaks this requirement when it spawns `modn` (with `Command::new`,
`CARGO_BIN_EXE_modn` or `main_bucket::modn_binary()`) and runs a command that
can reach bucket, module, content-root or install state without setting
`LOCALAPPDATA` to a temporary folder, or when the command can reach a module
with a content root without also setting `APPDATA` (or `MODN_APPDATA`). In
core tests, code that reaches `data_directory()` must run inside
`data_dir::with_data_directory` with a temporary folder. Commands that only
print help or fail argument parsing before opening state are exempt.

`main_bucket::command(...)` in `tests/support/main_bucket.rs` sets
`LOCALAPPDATA` to a seeded temporary folder; it does not set `APPDATA`.

Why it matters: these tests run on developer Windows machines where
`%LOCALAPPDATA%\modn` and `%APPDATA%` hold real trust records, module
registries and game mod folders. A missed variable lets a test overwrite the
user's state, and also makes the test depend on whatever is already there.
This was found during Milestone 5 and recorded in project memory
(`memory/events/20261109T135908Z-PM-20261109-eef46ab9.md`).

Incorrect: module and content-root state go to the real folders.

```rust
fn cli(state: &Path, args: &[&str]) -> Output {
    Command::new(main_bucket::modn_binary())
        .arg("--state-dir")
        .arg(state)
        .args(args)
        .output()
        .expect("run modn")
}
```

Correct: every user-data location is a temporary folder
(`apps/modn-cli/tests/module_state_dir_cli.rs`).

```rust
fn cli(data: &Path, state_dir: Option<&Path>, args: &[&str]) -> Output {
    let mut command = Command::new(main_bucket::modn_binary());
    command
        .env("LOCALAPPDATA", data)
        .env("APPDATA", data)
        .env_remove("MODN_GAME_MODULES_DIR")
        .env_remove("MODN_APPDATA");
    if let Some(state_dir) = state_dir {
        command.arg("--state-dir").arg(state_dir);
    }
    command.args(args).output().expect("run modn")
}
```
