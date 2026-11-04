# Modn vision

This page states the product direction. It changes only when that direction
changes. Planned work is tracked in GitHub milestones and issues; shipped
behavior is in [README.md](README.md).

## Product direction

Modn is a game-agnostic Rust mod manager using first-party Lua 5.4 game
modules. Each supported game's module lives in the official `main` bucket at
`games/<game-id>/module.json` and `module.lua`. Modn embeds no bucket manifests
or game modules. It bootstraps a verified local `main` copy on first use.

The `main` official bucket and registered local community buckets describe
mod packages: versions, hashes, sources and known dependencies. Only `main`
may contain game modules. The first bootstrap needs network access; the local
copy works offline afterward. Only `modn bucket update` refreshes it.
Third-party game modules (#51) and Git-backed catalog sync (#53) remain deferred.
Package manifests never authorize changes to game files: a
manifest may declare the tool executables its archive installs, but only the
user's confirmation or an explicit `--allow-executable` approves them.
Modn downloads `https` sources into a verified cache; `manual` sources still
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
