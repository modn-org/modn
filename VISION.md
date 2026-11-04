# Modn vision

This page states the product direction. It changes only when that direction
changes. Planned work is tracked in GitHub milestones and issues; shipped
behavior is in [README.md](README.md).

## Product direction

Modn is a game-agnostic Rust mod manager using first-party Lua 5.4 game
modules. Each supported game's module lives in the official `main` bucket at
`games/<game-id>/module.json` and `module.lua`. Modn embeds no bucket manifests
or game modules. The install script runs `modn bucket update` to install a local `main` copy.

The `main` official bucket and registered local community buckets describe
mod packages: versions, hashes, sources and known dependencies. Only `main`
may contain game modules. The local copy works offline. Only `modn bucket update` fetches it.
If the copy is missing, Modn asks the user to run that command.
The update checks the ZIP against its release SHA-256 sidecar from the same origin.
Release signing is future work. A separate `modn-org/main` repository is planned in #69.
The CLI supports explicit trusted module registration and community Git catalog sync.
Third-party Lua modules use a separate copied registry; community catalogs contain metadata only.
Package manifests never authorize changes to game files: a
manifest may declare the tool executables its archive installs, but only the
user's confirmation or an explicit `--allow-executable` approves them.
Modn downloads `https` and pinned `github-release` sources into a verified cache; `manual` sources still
need the user's file.

Development-only `--module-src` (`--src`) and `MODN_GAME_MODULES_DIR`
overrides load source explicitly selected for the current command and are
never persisted. A missing or mismatched override fails closed without
choosing a fallback.

Rust owns installation transaction state, conflict protection, file and
directory ownership, rollback and recovery. Lua provides game-specific
discovery, diagnostics and installation proposals; it does not bypass Rust's
install transaction guarantees. Prefer clean installs, not automatic adoption
of manually installed mods.

Windows comes first.
MCP, desktop UI, asset workflows, REDmod deployment and automatic cleanup remain pending.
Skyrim support has synthetic fixture coverage, without real-game validation.

## Where things live

- **Shipped behavior:** [README.md](README.md), updated in the same change as the code.
- **Planned work:** [GitHub milestones](https://github.com/sooskca/modn/milestones)
  and issues. Each milestone description holds its exit gate. A PR closes its
  issue with `Fixes #n`.
- **Deferred work:** issues with the
  [`deferred`](https://github.com/sooskca/modn/labels/deferred) label. Each one
  needs a separate proposal and tests before work starts.
- **Decisions, findings and failures:** project memory in `memory/`. When a
  milestone closes, record its result and evidence there as a finding.
