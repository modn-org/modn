# Project documentation

Start with the document for your task. These pages describe shipped behavior.
Planned work is in GitHub milestones and issues. A plan does not override
shipped behavior.

## User guides

- [`../README.md`](../README.md): what Modn does, installation and a quick start.
- [`install.md`](install.md): install Modn on Windows and verify the download.
- [`getting-started.md`](getting-started.md): first-time setup, game discovery,
  the state directory and a clean manager state.
- [`cli.md`](cli.md): command reference for games, buckets, search, install,
  update, list and uninstall.
- [`installing.md`](installing.md): catalog and standalone installs, dependency
  plans, stacks and tool executable approval.
- [`updating.md`](updating.md): pin, unpin and update a managed package.
- [`cache.md`](cache.md): the verified download cache and `modn cache`.
- [`inventory.md`](inventory.md): `scan` and `doctor`, ownership drift and
  framework signals.
- [`assets.md`](assets.md): read-only DDS and RDAR structure inspection.
- [`archives.md`](archives.md): `modn inspect` and `install --dry-run` planning.
- [`buckets.md`](buckets.md): the official `main` bucket, `modn bucket update`,
  local buckets and manifest rules.
- [`cyberpunk.md`](cyberpunk.md): Cyberpunk 2077 detection, supported package
  layouts and framework installation.

## Engineering reference

- [`../AGENTS.md`](../AGENTS.md): development rules and required checks.
- [`vision.md`](vision.md): approved product direction and where planned work
  lives.
- [`glossary.md`](glossary.md): canonical names for bucket, module, mod,
  archive and ownership concepts.
- [`data-model.md`](data-model.md): entities, trust, selection and ownership
  boundaries, and their mapping to persistence.
- [`game-modules.md`](game-modules.md): Lua game modules, `modn module check`,
  host API v1, read-only capabilities and development overrides.
- [`transactions.md`](transactions.md): transactional install, exclusive
  folders, created directories and recovery.
- [`development.md`](development.md): build, test, `verify-modn` and the
  read-only Cyberpunk archive benchmark.
- [`releasing.md`](releasing.md): the `Release` workflow, the release gates and
  real-game validation.

## Buckets and verification

- [`../buckets/README.md`](../buckets/README.md): bucket metadata contract v1.
- [`modn-org/main`](https://github.com/modn-org/main): the `main` bucket
  repository. Its `PROVENANCE.md` holds versions, checksums and dependency
  evidence. This repository pins it as the `buckets/main` submodule.
- [`../.agents/skills/verify-modn/SKILL.md`](../.agents/skills/verify-modn/SKILL.md):
  end-to-end CLI verification with disposable fixtures.
- [`../.agents/skills/verify-ownership/SKILL.md`](../.agents/skills/verify-ownership/SKILL.md):
  property checks for destination validation and ownership.

When a page disagrees with the code, fix the page in the same change.

## Additional interfaces and lifecycle

- [sources.md](sources.md): Git catalogs, verified cache reuse and GitHub release sources.
- [lifecycle.md](lifecycle.md): REDmod content, explicit adoption and approved cleanup.
- [mcp.md](mcp.md): scoped stdio companion, approval and limits.
- [desktop.md](desktop.md): Windows GPUI application, setup, operation and limits.
