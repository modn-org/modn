# Modn roadmap

Status: product direction, 2026-10-31. This document describes **decisions and
future milestones**, not a claim that they are implemented. See [README.md](README.md)
for commands that work on the checked-out revision, [data-model.md](data-model.md)
for proposed entities and invariants, and [glossary.md](glossary.md) for terms.

## Product direction

Modn is a game-agnostic Rust mod manager with game-specific **Lua** modules
distributed through **buckets**. Cyberpunk 2077 is the first real integration;
adding another game should not require a Rust adapter or a core rebuild.

The **official main bucket** is maintained and curated by the Modn project. It
distributes both game modules and mod/package manifests. Modn trusts this bucket
by default. Users can explicitly add community buckets; one clear warning
explains that a community bucket may supply executable game modules with broad
local capabilities. Trust is granted to the bucket maintainer, not requested
again for each ordinary module operation. Users can inspect origin, disable
modules, or remove bucket registration.

Trust is a **publisher/maintainer decision**, not a proof of harmlessness.
Trusted Lua modules should be powerful enough to integrate games and tools.
Do not build elaborate per-capability prompts or a hostile-code sandbox as the
primary product model. Keep installation correctness independent of this choice:
the normal managed-install path still uses Rust's ownership, conflict checks,
transaction journal, rollback and recovery. A trusted module does not silently
acquire ownership of pre-existing user files.

Prefer **standard Lua 5.4** via `mlua` for the eventual module runtime. Use
Lua 5.4 as the single supported runtime, not two parallel script engines.
Preserve established module contracts or document a deliberate version transition.

Keep the product **bucket-first**. Source URLs and user-supplied local archives
are enough for initial acquisition. Do not introduce Nexus, DEG Mods, BasedMods,
Modding Haven or other site-specific discovery/download providers now. The
official bucket should reference original distributions and record provenance;
it should not redistribute upstream binaries without permission.

## Current baseline and gaps

On `main` at the time of writing:
- Rust CLI, SQLite state, discovery, diagnostics, ZIP/7z previews and approved
  installs, file/directory ownership, uninstall and crash recovery exist.
- Runtime-loaded, individually packaged **Lua 5.4** modules provide Cyberpunk
  policy; a synthetic second-game module proves the host is game-agnostic.
- Module versions/selections live in a separate manager-owned module store.
  They are **not yet sourced from buckets**.
- The full eleven-mod prerequisite stack has been exercised in disposable
  installation fixtures, **not verified end to end in the live game**.
- Clean-install-first is intentional. Implicit adoption and cleanup of manual
  mods are **not** part of the MVP.

**PR #23 (Bucket Foundation) merged into `main`** on 2026-10-31. It
introduced local bucket registration, metadata lookup, and an embedded
`main` snapshot of CET, RED4ext and redscript. The implemented package
layout is `games/<game-id>/<package-id>.json`. This is not yet a separate
official bucket Git repository, game-module distribution, persisted community
trust, or dependency-aware installation. Reconcile the schema with the target
below **before declaring it stable**.

The historical October 30 roadmap handoff led to the existing Rust/Luau
implementation. This document supersedes its **ordering and trust/runtime
assumptions**, not its verified transaction or ownership requirements.

## Milestones (in dependency order)

Each milestone is a separate, scoped change. PR numbers beyond #23 are not
reserved. Do not merge, change live game files, or publish releases merely
because a milestone is listed here.

### 1. Bucket contract and registration

- Finish the metadata-only bucket foundation, then specify one stable on-disk
  and repository format for **both** package manifests and game modules.
- Provide the built-in official `main` bucket as a bootstrap snapshot, and
  define how its independently maintained Git repository is selected and
  updated. Do not require a network connection to start Modn.
- Support explicit local and Git-backed community buckets, namespaced IDs,
  schema-version checks, deterministic lookup and clear origin display.
- Record a one-time **community bucket trust acknowledgment** before enabling
  its executable modules. Official `main` needs no repeated prompts.
- Keep bucket metadata distinct from installed/owned state. No catalog command
  should mutate a game installation.

**Exit gate:** valid and malformed bucket fixtures behave predictably on
Windows and Linux; duplicate package names remain unambiguous; a community
bucket cannot become trusted silently.

### 2. Bucket-managed game modules and Lua 5.4

- Put each game's versioned module inside the official or selected community
  bucket. A module can coexist with package manifests for the same game.
- Maintain the standard Lua 5.4 runtime, converted Cyberpunk/test-game
  scripts and distributed module packaging (runtime migration in PR #24).
- Use a clear **active module selection** per game, with provenance, version,
  integrity check, explicit update/disable and rollback. Do not replace the
  active module just because a bucket changes.
- Make game-specific discovery, layouts, compatibility rules, diagnostics
  and external-tool integration the Lua module's responsibility. Keep generic
  installation/state mechanisms in Rust.
- Favor practical APIs for trusted modules instead of layered permissions.
  When a module uses the managed installer, it must still respect the Rust
  transaction and ownership contract.

**Exit gate:** the official Cyberpunk module loads from a bucket, reproduces
current diagnostics/install proposals, and can be updated/rolled back. An
independent fictional module still runs without adding game-specific Rust.

### 3. Official Cyberpunk catalog

- Maintain one reviewed `main` bucket, ideally in its own Git repository.
  Ship an offline-compatible snapshot with Modn until remote refresh is proven.
- Curate the eleven original prerequisite packages: RED4ext, Cyber Engine
  Tweaks (CET), redscript, ArchiveXL, TweakXL, Codeware, Browser Extension,
  Equipment-EX, Mod Settings, No Intro Videos and Virtual Atelier.
- Include exact versions, game requirements, original source URLs or manual
  download instructions, archive hashes, known dependencies and compatibility
  notes. **Do not infer undocumented dependencies or a verified game boot.**
- Review changes to executable game modules and to manifests in the official
  bucket before publication. An official listing is a maintainer endorsement,
  not a promise that arbitrary mod binaries are risk-free.

**Exit gate:** all catalog entries match inspected upstream distributions and
identify their verified source or required user-provided archive.

### 4. Dependency planning and archive acquisition

- Resolve required/optional/conflicting relationships deterministically.
  Handle cycles, missing entries, duplicate IDs, incompatible versions and
  already-installed versions with clear diagnostics.
- Plan a complete install before writing: selected bucket snapshots, module,
  package versions, source checksums, destinations, dependencies and conflicts.
- Fetch ordinary pinned HTTPS artifacts into a verified local cache; support
  importing existing ZIP/7z archives by digest when automated fetching is
  unavailable. No provider-specific integration is required.
- Install through existing generic Rust transactions after user approval. Do
  not auto-approve a changed dependency graph, overwrite unmanaged files, or
  automatically update an active framework/module.

**Exit gate:** a disposable full-stack recipe resolves from a bucket, verifies
archive bytes, and survives failure/rollback without ownership drift.

### 5. Controlled Cyberpunk live-game validation

- Only after explicit permission for **each live-game testing scope**, start
  with a verified clean installation and backups of saves/configuration.
- Install the curated prerequisite stack through the user-facing bucket path,
  in dependency order; launch the game between meaningful checkpoints.
- Confirm runtime loading, diagnostics, dependency behavior and uninstall in
  reverse order. Protect unmanaged content and game-generated caches.
- Repair general layout concepts rather than adding mod-by-mod exceptions.

**Exit gate:** real game boots and intended mod behavior works, installation
inventory matches the files created, uninstall preserves unrelated files, and
crash-recovery behavior remains exercised in disposable fixtures.

### 6. Usable Windows-first alpha and package lifecycle

- Support explicit package update/replacement previews, version pins,
  rollback and integrity/ownership drift diagnostics.
- Package a portable Windows build with the default official bucket snapshot,
  Lua runtime, useful errors and clean-state setup guidance.
- Present source bucket, module version, mod provenance and trust state
  clearly in the CLI; add a simple package browser if needed.

**Exit gate:** a fresh Windows installation works without developer tooling,
and a failed install or update has a documented recovery path.

## Deferred work, not implicit commitments

- REDmod deployment, Cyberpunk-specific asset inspection (such as red4lib),
  and additional archive formats each need separate requirements and tests.
- Add a second **real** game to prove the bucket + Lua architecture is generic;
  the synthetic fixture alone is not production compatibility evidence.
- Optional game-agnostic deduplication requires content identity, reference
  accounting and safe garbage collection before it can be enabled.
- MCP, desktop UI, automated external-provider discovery, public Nexus API
  integration, cloud catalogs and advanced cross-platform releases come later.
- Manual-mod adoption, automatic game repair and forced cleanup are excluded
  from the clean-install-first MVP unless explicitly revisited.

## Working agreement

Consult [AGENTS.md](AGENTS.md) before modifying the project. Check actual
Git/PR state and the [README](README.md) before claiming a milestone is done.
Use a separate worktree and focused tests for each implementation. Keep
implementation evidence in the PR and final response, not unsolicited logs.
A roadmap entry does not authorize a merge or a real game modification.
