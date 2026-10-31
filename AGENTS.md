# modn development instructions

## Authority and scope

- Follow the project's architecture and behavior described in README.md.
- Use the [Google Rust Style Guide](https://google.github.io/styleguide/rustguide.html) and the Gate `google-rust-style` skill for new or changed Rust code, subject to this file and existing correctness, safety, and API contracts.
- Use the official Rust API Guidelines where neither project instructions nor Google give a rule. Keep established local conventions unless a change has a concrete benefit.
- Google rules specifically for Chromium, Fuchsia, and Android do not apply. This workspace uses stable Rust; nightly-only rustfmt options are review guidelines, not build requirements.

## Project domain documentation

Read these documents before substantive changes:

1. [README.md](README.md)  - implemented features, commands, tested behavior and current limitations.
2. [roadmap.md](roadmap.md)  - approved bucket-first product direction, milestones and explicit deferrals.
3. [glossary.md](glossary.md)  - canonical names for bucket, module, mod, archive and ownership concepts.
4. [data-model.md](data-model.md)  - conceptual entities, trust/selection/ownership boundaries and mapping to existing persistence.

The roadmap and data model distinguish **planned** capabilities from shipped code. When
they differ from the code, inspect the current revision and active PRs; do not
present planned behavior as implemented. Use glossary terms consistently in
code, documentation, CLI and reviews.

The target design has game-specific modules distributed through the official
trusted bucket, with community buckets explicitly trusted by the user. Standard
Lua 5.4 is the game-module runtime in this revision; earlier releases used Luau.
A trusted module may be powerful, but Rust-owned managed transactions,
ownership and recoverability are separate correctness guarantees. No
roadmap item authorizes modifying a user's real game or merging a PR.

## Rust implementation

- Prefer the simplest correct design: unique ownership and borrowing before `Rc`/`Arc`; inherent methods before unnecessary traits, and functions before macros.
- For unclear boolean arguments, prefer meaningful enums. Use newtypes for semantically distinct identifiers. Favor named struct fields.
- Use `Option` for expected absence, `Result` for recoverable errors, and `expect("invariant")` rather than `unwrap()` when a true programmer invariant must be asserted. Never panic for ordinary malformed mod input or recoverable filesystem failures.
- Keep game-agnostic functionality in the core and game-specific behavior in game modules. Keep host APIs focused on stable capabilities, not implementation details; use Lua 5.4 via mlua, keeping game policy outside Rust.
- Preserve transactional installation, ownership tracking, rollback, and recovery guarantees. Do not make code prettier at the cost of recoverability.
- Public libraries should apply Google's additional API/documentation guidelines; application-only code can prefer directness over general-purpose abstractions.
- Keep source files at or below 500 lines when creating or modifying them; split an existing oversized file before making substantive changes to it.
- Use `#[expect(clippy::lint_name, reason = "...")]` for a justified local exception rather than broadly disabling a lint.

## Review and verification

- Do not introduce unrelated refactors for style-only changes. Apply conventions to new or touched code without changing unrelated behavior.
- Run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, and `cargo test --workspace --locked` for Rust changes.
- Inspect the Git state before editing and use an isolated worktree for changes; preserve unrelated changes and other worktrees. Do not merge without authorization.
