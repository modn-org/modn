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
Modn downloads `https` sources into a verified cache; `manual` sources still
need the user's file. Git-backed catalog sync remains a future milestone.

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
- The milestone 3 exit gate passes. `apps/modn-cli/tests/milestone_gate.rs`
  checks the full eleven-mod stack in a disposable fake game directory, through
  the user-facing install path. The first check is that a failed stack rolls
  back. The second is that recovery rolls back an interrupted stack. The third
  is that all 11 packages (137 owned files) install and uninstall in reverse
  order to the original tree, directories included. A synthetic copy of the graph runs in CI. The
  real run needs the four Nexus archives.
- Milestone 4 passed on a real Steam Cyberpunk 2077 installation, 2026-11-02;
  see milestone 4 below.
- `modn game add/list/remove` registers games; `modn install` handles standalone archives and verified catalog mods; `modn search` lists catalog entries; `modn list` shows managed packages.
- `modn install <mod> --dry-run` shows a read-only dependency plan: install
  order and the state of each dependency. `modn install <mod> --file ...`
  installs the mod and its missing dependencies from supplied archives as one
  approved stack that rolls back as a whole. Missing `https` archives download
  into a SHA-256-verified cache, after Scoop's. Manifests may declare
  dependency version requirements and conflicts, which block a plan before
  any write. Choosing between several releases of one mod and the full catalog
  remain future milestones.

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
Met by `milestone_gate.rs`; see the baseline above.

### 4. Controlled live Cyberpunk validation

Only after explicit authorization for each live scope, begin with a clean
verified game installation and backups. Install the prerequisite stack through
the user-facing path, launch the game at meaningful checkpoints, validate
runtime behavior and uninstall in reverse order.

**Exit gate:** each checkpoint installs and launches without errors, and the
full uninstall leaves only runtime files the game created.

**Met on 2026-11-02** on a Steam installation that reports game version
3.0.88.46235. The user authorized each live step separately and launched
the game; the agent never launched it. The backup was a SHA-256 manifest of
the game tree, so Steam file verification was the only restore path. The
user checked Steam integrity before the run.

| Checkpoint | Packages added through `modn install` |
|---|---|
| a | red4ext 1.31.0, cet 1.38.1 |
| b | redscript 0.5.32, codeware 1.21.2 |
| c | virtual-atelier 1.6.4, with archivexl 1.28.6 and mod-settings 0.2.28 |
| d | equipment-ex 1.2.10 with tweakxl 1.12.0, then browser-extension 0.9.7, then no-intro-videos 0.8 |

- After every install, a diff against the manifest showed only owned files
  added. No vanilla file was modified or removed.
- After checkpoint a, the user reported that the CET overlay opened and a
  save loaded. After b, the user reached the main menu. After c and d, the
  user reported that everything looked fine, but no specific in-game feature
  checks. For a and b, runtime logs showed RED4ext, CET, redscript and
  Codeware loading without errors.
- The full stack held 11 packages and 137 owned files, as in the milestone 3
  gate. `modn uninstall` removed them in reverse order.
- Afterwards the diff showed only runtime files: 21 added files, 7 added
  folders, a rewritten `r6/logs/redscript_rCURRENT.log`, and 4 older
  redscript logs that redscript's own rotation deleted. The user chose to keep
  these files. Modn keeps the `red4ext` and `bin/x64/plugins/cyber_engine_tweaks`
  folder records while runtime files remain in them.
- The run found three Modn gaps: uninstall by catalog name and its preview,
  the hidden created-folder ledger, and dry runs that could not plan
  uncached `https` archives. PRs #45 and #46 fixed them.

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
