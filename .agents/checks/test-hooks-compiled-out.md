---
files:
  - "crates/*/src/**/*.rs"
  - "apps/*/src/**/*.rs"
  - "**/Cargo.toml"
  - "!**/tests.rs"
  - "!**/*_tests.rs"
  - "!**/*_tests/**"
  - "!**/tests/**"
---

# Test-only overrides are compiled out of release builds

Tests drive Modn through hooks that a released `modn.exe` must not have:
environment variables such as `MODN_TEST_MAIN_BUCKET_PUBLIC_KEY`,
`MODN_TEST_GIT_REMOTE_BASE`, `MODN_TEST_UPDATE_FAULT_AFTER` and
`MODN_APPDATA`, injected faults, and changed execution budgets. Each such hook
must sit behind `#[cfg(feature = "test-main-key")]`,
`#[cfg(feature = "test-instruction-budget")]` or `#[cfg(test)]` (alone or in
`any(...)`), with a `#[cfg(not(...))]` counterpart that uses the production
behavior and reads no override. These `modn-core` features may be enabled
only under `[dev-dependencies]`; Cargo unifies them into the binaries that
integration tests spawn, while a normal build of the app has none of them.

The requirement is broken when a change:

- reads a new test-only environment variable, or adds a fault, timing or
  budget hook, in code that is compiled into a normal build;
- moves an existing hook out of its `cfg` gate, or widens the gate so a
  normal build enables it;
- enables `test-main-key` or `test-instruction-budget` in `[dependencies]`,
  in `default` features, or from another feature a release build turns on.

Documented user overrides such as `MODN_CACHE`, `MODN_MAIN_BUCKET_SOURCE` and
the developer override `MODN_GAME_MODULES_DIR` are product behavior described
in `docs/` and are not covered here. A new override counts as test-only unless
the same change documents it in `docs/`.

Why it matters: these hooks bypass trust and recovery guarantees. An ungated
`MODN_TEST_MAIN_BUCKET_PUBLIC_KEY` would let anyone who can set an environment
variable replace the embedded main bucket signer and serve an unsigned
catalog with game modules; an ungated remote base or fault hook redirects Git
sources or breaks updates on purpose. The feature unification that makes
these hooks reach CLI tests is recorded in
`memory/events/20261107T145946Z-PM-20261107-48d701d5.md`.

Incorrect: the override ships in every build.

```rust
fn main_bucket_key_text(embedded: &str) -> Cow<'_, str> {
    match std::env::var("MODN_TEST_MAIN_BUCKET_PUBLIC_KEY") {
        Ok(text) if !text.trim().is_empty() => Cow::Owned(text),
        _ => Cow::Borrowed(embedded),
    }
}
```

Correct: the override exists only with the test feature
(`crates/modn-core/src/buckets/signature.rs`).

```rust
#[cfg(feature = "test-main-key")]
fn main_bucket_key_text(embedded: &str) -> Cow<'_, str> {
    match std::env::var("MODN_TEST_MAIN_BUCKET_PUBLIC_KEY") {
        Ok(text) if !text.trim().is_empty() => Cow::Owned(text),
        _ => Cow::Borrowed(embedded),
    }
}

#[cfg(not(feature = "test-main-key"))]
fn main_bucket_key_text(embedded: &str) -> Cow<'_, str> {
    Cow::Borrowed(embedded)
}
```

In `apps/modn-cli/Cargo.toml` the features belong only to the dev-dependency:

```toml
[dependencies]
modn-core = { path = "../../crates/modn-core" }

[dev-dependencies]
modn-core = { path = "../../crates/modn-core", features = ["test-instruction-budget", "test-main-key"] }
```
