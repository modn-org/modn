# modn

Modn is a mod manager for Cyberpunk 2077 on Windows. Its Rust core is
game-agnostic. A Lua module in the official `main` bucket supplies the rules for
each game.

## What does modn do?

- finds a registered game and reports the mods on disk, managed or not
- installs ZIP and 7z mod packages from a bucket or a local file, after a dry
  run that you can review
- records each file it writes, so uninstall, update and rollback remove only
  what Modn owns
- recovers an interrupted transaction with `modn recover --yes`
- never claims or overwrites files that it did not install

Modn supports legacy `.archive`, CET, RED4ext, redscript, ArchiveXL and TweakXL
packages. REDmod deployment, other games, MCP and a desktop UI are not
implemented.

## Installation

Run this command in PowerShell without administrator rights:

```powershell
irm https://github.com/modn-org/modn/releases/latest/download/install.ps1 | iex
```

The script installs `modn.exe`, adds it to your user `PATH` and runs
`modn bucket update`. Open a new terminal before you use `modn`. To verify the
download, see [`docs/install.md`](docs/install.md).

## Quick start

Start from a clean, verified game installation. Then:

```powershell
modn game add cyberpunk-2077 --path "C:\Games\Cyberpunk 2077"
modn doctor --path "C:\Games\Cyberpunk 2077"
modn search cet
modn install cet --dry-run
modn install cet
modn list
modn uninstall cet --yes
```

[`docs/getting-started.md`](docs/getting-started.md) explains each step.

## Buckets

Like Scoop, Modn reads mod manifests from buckets. The official `main` bucket
holds the first-party game modules and the Cyberpunk catalog. Add a local bucket
with `modn bucket add <name> <directory>`. See [`docs/buckets.md`](docs/buckets.md).

More docs are in [`docs/`](docs/README.md).

## Contributing

Planned work is in GitHub milestones and issues. To report a bug or propose an
idea, [open an issue](https://github.com/modn-org/modn/issues). To build and
test Modn, see [`docs/development.md`](docs/development.md).

MIT licensed.
