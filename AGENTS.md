# modn development instructions

## Authority and scope

- Follow the project's architecture and behavior described in [docs/](docs/README.md).
- Use the [Google Rust Style Guide](https://google.github.io/styleguide/rustguide.html) and the Gate `google-rust-style` skill for new or changed Rust code, subject to this file and existing correctness, safety, and API contracts.
- Use the official Rust API Guidelines where neither project instructions nor Google give a rule. Keep established local conventions unless a change has a concrete benefit.
- Google rules specifically for Chromium, Fuchsia, and Android do not apply. This workspace uses stable Rust; nightly-only rustfmt options are review guidelines, not build requirements.

## Project domain documentation

Read these documents before substantive changes:

1. [docs/README.md](docs/README.md)  - index of the user guides that describe implemented features, commands, tested behavior and current limitations; read the guides for the area you change.
2. [docs/vision.md](docs/vision.md)  - approved bucket-first product direction and where planned work is tracked.
3. [docs/glossary.md](docs/glossary.md)  - canonical names for bucket, module, mod, archive and ownership concepts.
4. [docs/data-model.md](docs/data-model.md)  - conceptual entities, trust/selection/ownership boundaries and mapping to existing persistence.

Keep [README.md](README.md) short: what Modn does, installation, a quick start and a link to `docs/`. Put shipped-behavior detail in the matching `docs/` page, in the same change as the code.

Planned work lives in GitHub milestones and issues (`gh issue list`,
`gh api repos/modn-org/modn/milestones`); each milestone description holds its
exit gate, and the `deferred` label marks out-of-scope work. Close an issue
from its PR with `Fixes #n`. When a milestone closes, record its result and
evidence in project memory. The data model and issues describe **planned**
capabilities; when they differ from the code, inspect the current revision
and active PRs, and do not present planned behavior as implemented. Use glossary terms consistently in
code, documentation, CLI and reviews.

Project memory lives in `memory/events/`, one immutable record file per
durable decision, finding, failure or preference. Use the `project-wiki` skill
to look up records at the start of substantive work, to add records, and to
supersede them. Commit each record with the change it describes. Push a
memory-only record directly to `main`; do not open a pull request for it. When memory
disagrees with the documents above or with the code, the documents and the
code win: supersede the stale record.

First-party game modules live beside mod manifests in the official `main` bucket; registered local buckets cannot contain modules. Modn embeds no bucket manifests or game modules. `modn bucket update` is the only command that fetches the local `main` copy; the install script runs it once. Standard Lua 5.4 via mlua is the game-module runtime; earlier releases used Luau.
A trusted module may be powerful, but Rust-owned managed transactions,
ownership and recoverability are separate correctness guarantees. No
milestone or issue authorizes modifying a user's real game or merging a PR.

Desktop page designs live in `design/` as FigCraft files. The owner owns
`design/design-system.figcraft`. For any design request, follow
`.agents/skills/design-modn/SKILL.md`.

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
- For changes to destination validation, ownership keys, install, uninstall or `install --dry-run` (files listed in its coverage record), follow `.agents/skills/verify-ownership/SKILL.md`: run `cargo test -p modn-core --locked --lib destination_properties -- --include-ignored` and `cargo test -p modn-cli --locked --test ownership_properties -- --include-ignored`.
- Inspect the Git state before editing and use an isolated worktree for changes; preserve unrelated changes and other worktrees. Do not merge without authorization.
