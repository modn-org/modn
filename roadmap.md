# Modn roadmap

Status: product direction, 2026-10-31. This document describes planned
milestones, not necessarily implemented behavior. See [README.md](README.md)
for working commands, [data-model.md](data-model.md) for invariants and
[glossary.md](glossary.md) for terminology.

## Product direction

Modn is a game-agnostic Rust mod manager using first-party Lua 5.4 game
modules. Every supported game has a canonical source at
`crates/modn-core/game-modules/<game-id>/`. Modn reviews these sources and
compiles them into the executable. They are not downloaded, installed, activated,
or updated through buckets.

The `main` official bucket and registered local community buckets describe
mod packages only: versions, hashes, sources and known dependencies.
Buckets cannot publish executable Lua. The official `main` package catalog
works offline. Package manifests never authorize changes to game files: a
manifest may declare the tool executables its archive installs, but only the
user's confirmation or an explicit `--allow-executable` approves them.
Git-backed catalog sync and acquisition remain future milestones.

Development-only `--module-src` (`--src`) and
`MODN_GAME_MODULES_DIR` overrides load source explicitly selected for the
current command and are never persisted. A missing or mismatched override
fails closed without choosing a fallback.

Rust owns installation transaction state, conflict protection, file and
directory ownership, rollback and recovery. Lua provides game-specific
discovery, diagnostics and installation proposals; it does not bypass Rust's
install transaction guarantees. Prefer clean installs, not automatic adoption
of manually installed mods. Third-party game-module distribution is deferred
until there is a real requirement.

## Current baseline and gaps

- Game-agnostic Rust CLI, discovery, ZIP/7z metadata previews, approved
  installs, ownership tracking, uninstall and crash recovery.
- First-party Cyberpunk Lua 5.4 runtime policy and a synthetic second-game
  fixture. Lua host capabilities remain scoped and bounded.
- Package-only official `main` bucket snapshot with all eleven prerequisite
  manifests, verified against their original distributions, and explicit
  local bucket registration.
- The full eleven-mod prerequisite stack has been exercised in disposable
  fixtures, not verified end to end in the live game.
- `modn game add/list/remove` registers games; `modn install` handles standalone archives and verified catalog mods; `modn search` lists catalog entries; `modn list` shows managed packages.
- `modn install <mod> --dry-run` shows a read-only dependency plan: install
  order and the state of each dependency. `modn install <mod> --file ...`
  installs the mod and its missing dependencies from supplied archives as one
  approved stack that rolls back as a whole. Version constraints, artifact
  downloads and the full catalog remain future milestones.

## Milestones (in dependency order)

### 1. First-party Lua module simplification

Remove duplicate Cyberpunk source copies, bucket module catalog entries
and the separate ZIP module installation/activation database. Keep one Lua
source per supported game in Modn and a temporary development override.
Preserve current discovery, diagnostics and safe installation proposals.

**Exit gate:** Cyberpunk policy loads from the embedded first-party module,
overrides do not persist or fall back silently, and an independent fictional
module runs without game-specific Rust.

### 2. Official Cyberpunk package catalog

Curate original prerequisite distributions: RED4ext, Cyber Engine Tweaks,
redscript, ArchiveXL, TweakXL, Codeware, Browser Extension, Equipment-EX,
Mod Settings, No Intro Videos and Virtual Atelier. Record verified versions,
source URLs or manual download requirements, hashes, dependencies and
compatibility evidence. Do not redistribute upstream binaries without permission.

**Exit gate:** all entries match inspected original distributions.

### 3. Dependency planning and acquisition

The unified install command resolves catalog mods, validates manually acquired archives and requires explicit approval. Game registration remains independent. Continue with dependency graph resolution and verified HTTPS acquisition.



Continue with dependency graph solving, compatible versions and conflicts,
then pinned HTTPS archive acquisition and verified caching. Unresolved
dependencies must remain blocked until the planner supports them. Preserve
the existing deterministic `modn install` resolver, read-only preview and
explicit install approval (interactive confirmation or `--yes`); never overwrite unmanaged files or
silently update active frameworks.

**Exit gate:** reproducible disposable full-stack installation and rollback.

### 4. Controlled live Cyberpunk validation

Only after explicit authorization for each live scope, begin with a clean
verified game installation and backups. Install the prerequisite stack through
the user-facing path, launch the game at meaningful checkpoints, validate
runtime behavior and uninstall in reverse order.

### 5. Windows-first alpha and package lifecycle

Ship a portable executable with the first-party Lua policy and official
offline package catalog. Support explicit package update/replacement previews,
pins, rollback, ownership drift diagnostics and clean-state setup guidance.

## Deferred work

Independent third-party Lua modules, module marketplaces, separate module
installation, module release pins/rollback and module-source bucket trust are
not current requirements. Additional real games, remote catalog sync, REDmod,
game asset inspection, deduplication, provider APIs, MCP and desktop UI require
separate proposals and tests. Manual-mod adoption and forced cleanup are
excluded from the clean-install-first MVP.

## Working agreement

Consult AGENTS.md before modifying code. Check actual Git/PR state, preserve
unrelated changes, use an isolated worktree, and test affected behavior.
Do not merge or modify live-game files without explicit authorization.
