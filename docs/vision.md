# Modn vision

This page states the product direction. It changes only when that direction
changes. Planned work is tracked in GitHub milestones and issues; shipped
behavior is in the [user guides](README.md#user-guides).

## Product direction

Modn is a game-agnostic Rust mod manager using first-party Lua 5.4 game
modules. Each supported game's module lives in the official `main` bucket at
`games/<game-id>/module.json` and `module.lua`. Modn embeds no bucket manifests
or game modules. The install script runs `modn bucket update` to install a local `main` copy.

The `main` official bucket and registered local community buckets describe
mod packages: versions, hashes, sources and known dependencies. Only `main`
may contain game modules. The local copy works offline. Only `modn bucket update` fetches it.
If the copy is missing, Modn asks the user to run that command.
The update verifies a minisign signature over the catalog digest with the signer
key built into Modn. Each release must include `main-bucket.zip.minisig`.
`main` lives in the `modn-org/main` repository, with its own release cycle.
Key rotation and revocation are planned (#122).
Trusted third-party game modules use a separate registry. Explicit Git-backed community catalog sync is available; see [sources.md](sources.md).
Package manifests never authorize changes to game files: a
manifest may declare the tool executables its archive installs, but only the
user's confirmation or an explicit `--allow-executable` approves them.
Modn downloads `https`, `github-release`, `git` and `nexus` sources into a
verified cache; `manual` sources still need the user's file.

Development-only `--module-src` (`--src`) and `MODN_GAME_MODULES_DIR`
overrides load source explicitly selected for the current command and are
never persisted. A missing or mismatched override fails closed without
choosing a fallback.

Rust owns installation transaction state, conflict protection, file and
directory ownership, rollback and recovery. Lua provides game-specific
discovery, diagnostics and installation proposals; it does not bypass Rust's
install transaction guarantees. Prefer clean installs. Explicit adoption requires acknowledgement and matching archive bytes;
see [lifecycle.md](lifecycle.md).

Windows comes first.

## Where things live

- **Shipped behavior:** [`docs/`](README.md) user guides, updated in the same change as the code.
- **Planned work:** [GitHub milestones](https://github.com/modn-org/modn/milestones)
  and issues. Each milestone description holds its exit gate. A PR closes its
  issue with `Fixes #n`.
- **Deferred work:** issues with the
  [`deferred`](https://github.com/modn-org/modn/labels/deferred) label. Each one
  needs a separate proposal and tests before work starts.
- **Decisions, findings and failures:** project memory in `memory/`. When a
  milestone closes, record its result and evidence there as a finding.
