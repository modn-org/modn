# Installing mods

## Catalog and standalone installs

Catalog installation needs each archive with the exact format and BLAKE3 pin
from its manifest. Modn downloads an `https`, `github-release` or `nexus`
source itself, or fetches a `git` source. A `manual` source is never
downloaded: supply it with `--file`, and without it Modn names the file to
get. Standalone
`modn install <archive.zip|archive.7z>` needs no catalog entry and uses
the existing Lua planner and Rust ownership transaction. `--dry-run` previews
without game writes. A catalog install in an interactive terminal shows what
it will write and asks `Install? [y/N]`; scripts and `--json` must pass
`--yes`. Standalone archive installs always require `--yes`. `--progress json`
streams phases and accepts a `cancel` line on stdin before the commit point;
see [cli.md](cli.md#progress-events-and-cancellation). Each catalog
dependency must be installed with its pinned digest and intact managed
content, or be installed in the same command. Installation re-checks this
while holding the transaction lock. Each catalog installation records its `bucket/game/mod` provenance and
the managed packages that satisfied its dependencies, in the same SQLite
commit as its ownership. `modn uninstall` refuses to remove a recorded
dependency while a dependent managed package remains; uninstall the dependent
first. Standalone archive installs and packages installed before this record
existed carry no dependency links.

## Dependency plan

`modn install <mod> --dry-run` also reports a read-only dependency plan.
It lists every dependency of the mod, transitively, in install order. The
plan resolves dependencies in the mod's own bucket and game. A cycle or an
unknown dependency ID fails the dry run. Each dependency has a state:
`ready` (managed with the pinned digest and intact), `damaged` (pinned digest
but drifted content), `other_release` (recorded from the same catalog mod
with another digest) or `missing`. Missing dependencies name their artifact
source. Without the archive, the dry run tells you how to get it: a
download-capable source says when you install it, and a `manual` source asks
for `--file`. JSON `plan` is `null`. JSON adds `dependencies` and `dependencies_ready`. Each
dependency also shows the supplied `archive`, if one matches. The plan
writes nothing.

## Download during a dry run

A dry run never downloads by default. Add `--download` to fetch the missing
archives of the mod and its missing dependencies into the verified
download cache first, including `https` and `nexus` sources. The dry run
then shows the file plan, and a later install reuses the cached archives.
`--download` writes only to the cache, never to the game, and applies only
with `--dry-run`. `manual` sources still need `--file`. A `nexus` source
needs an API key, and a free Nexus account also needs `--nxm`. See
[sources.md](sources.md).

## Install a stack

To install a mod with its missing dependencies, pass `--file` once for each
`manual` archive, in any order. Modn downloads or reuses the other sources.
A free Nexus account also passes the matching link with `--nxm` for each
Nexus download. Modn matches each archive to a release by BLAKE3 pin and refuses an archive that
no release in the installation pins. When a `manual` archive is missing, Modn
names it and writes nothing. One confirmation, or `--yes`, approves the whole stack. With `--yes`,
each executable that a stack manifest declares also needs `--allow-executable`,
unless a stored approval covers it.
Modn installs the stack in dependency order under one transaction lock. The
game policy plans each package after the packages before it are installed.
Each package keeps its own ownership record, provenance and dependency links.
If any package fails, Modn uninstalls the packages that this command
installed, newest first. Rollback, like `modn uninstall`, also removes the
parent directories that Modn created, once they are empty. Two
dependencies that pin the same archive digest install it once. If the process stops before the stack completes,
other writes are refused until `modn recover --yes` uninstalls the stack's
packages. A damaged dependency or another installed release is never replaced:
the install fails until you restore or uninstall it, or replace it with
`modn update`.

## Native-injector and executable approval

A catalog manifest sets `native_injector: true` when its archive places native
code in the game process. It may also declare the tool executables its archive
installs, for example redscript's `engine/tools/scc.exe`. A declaration only
lets Modn name them; it never approves them. Modn asks once per mod and game
installation. An install or update whose release capabilities the stored
approval covers proceeds without a flag. Uninstall keeps the approval, so a
reinstall of the same release needs no flag.

Modn stores an approval only when the operation commits. A cancelled, refused,
dry-run or rolled-back operation stores nothing. A release that drops a native
injector or an executable does not shrink the approval.

A release that adds a capability the stored approval does not cover is refused.
A new native injector returns `NATIVE_INJECTOR_NOT_ALLOWED`; a new executable
path returns `APPROVAL_REQUIRED`. The JSON error names the mod ID and the
uncovered capabilities. Pass `--allow-native-injector`, or
`--allow-executable <path>` for each new executable, to approve the release.
The successful operation widens the stored approval. An interactive terminal
can also confirm new executable paths. `--yes` never approves them by itself.

`install --dry-run --json` reports `approval.required`,
`approval.covered_by_stored_approval`, the stored approval and the uncovered
capabilities. An executable the manifest does not declare stays blocked unless
`--allow-executable` names it. Adoption and forced cleanup keep their own
explicit confirmation.

Each catalog install or update retains one bucket snapshot for release selection, dependency pins, conflicts and published versions. Bucket edits during a prompt do not change that command's selected evidence.

## Pinned release providers

Catalog manifests can select an exact GitHub repository, release tag and asset name with `github-release`.
A manifest can also pin a Nexus Mods file with `nexus`. It names the game domain, mod ID, file ID and file name.
The archive BLAKE3 pin and `archive_format` remain required for both kinds.
Ordinary previews make no provider request.
Explicit installation or `--dry-run --download` permits acquisition. See [sources.md](sources.md).
