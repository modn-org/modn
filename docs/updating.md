# Pinning and updating mods

`modn pin <selector>` prevents updates of one managed package.
`modn unpin <selector>` allows updates again. Both commands accept the
managed package ID, `mod`, `game/mod` or `bucket/game/mod`, as uninstall does.
Repeating either command succeeds without changing the pin.
`modn list` marks pinned packages; JSON includes a boolean `pinned` field.
Uninstall removes a pinned package and its pin normally.

An update skips a pinned managed package successfully without downloading
or changing the game, cache or manager state. Unpin it before updating.
A pinned dependent blocks a dependency update when its recorded requirement
refuses the target version. Modn checks this again under the transaction lock.

`modn update <mod>` replaces the installed release of a catalog mod with the
bucket's current release. `--version <version>` selects another release that
the bucket publishes. Name the mod as for install: `mod`, `game/mod` or
`bucket/game/mod`. The installed package must come from that catalog mod.

`modn update <mod> --dry-run` writes nothing. It shows these items:
- the installed package, its release version (when a release of the bucket
  has its archive digest) and its exclusive folders
- the target release
- the files that the update adds, replaces and removes
- each managed package that depends on the mod, and whether its installed
  release accepts the target version

The file list needs the target archive. Without it, the dry run names the
archive to supply. `--file` and `--download` work as for install. JSON adds
`up_to_date`, `blockers`, `plan` and `unapproved_declared_executables`.

An update needs `--yes` or an interactive confirmation, as for install. The
prompt names both releases and each dependent. Declared tool executables need
the same approval as for install.

Modn updates only the mod that you name. It never updates a dependency or a
framework as a side effect. The update is blocked in these cases:
- the target release fails the checks of an install: a dependency that the
  bucket does not publish, a `dependency_versions` requirement that the
  bucket's dependency release misses, a cycle or a conflict
- a dependency of the target release is not ready; update or install it first
- a dependent's installed release has a `dependency_versions` requirement
  that the target version does not meet
- a dependent's requirement is unknown
- a dependent needs the installed archive under another catalog name (an
  alias that pins the same digest); Modn cannot show that the new release
  satisfies that name
- the new release would depend, directly or through other packages, on a
  package that depends on the mod

Each catalog install records its dependency requirements with its ownership,
so an update checks the requirement of the release that is actually
installed, not of the bucket's current release. A package installed before
Modn recorded requirements falls back to the bucket release with its archive
digest. If the bucket no longer publishes that release, the requirement is
unknown: update or reinstall that dependent first.

The game policy plans the new release as if the old release were already
removed, so an exclusive folder is claimed again. Rust then validates the
proposal again on the real disk. The update runs in one recoverable
transaction under the transaction lock:
1. Modn moves the old release's files and exclusive folders aside, under a
   pending uninstall journal. Its ownership stays recorded.
2. Modn installs the new release with the usual checks. The old release's
   ownership does not block it. An unmanaged file still blocks it; Modn never
   overwrites one.
3. One SQLite commit retires the old package, records the new one, and moves
   each dependent's recorded dependency to the new package.

If a step fails before that commit, Modn restores the old release under its
package ID. If the process stops, other writes are refused until
`modn recover --yes`, which removes the new files and then restores the old
release. Before Modn links each new file into the game, it records the
attempt in the journal. Recovery deletes a private copy without checking it
only when no such record exists, because no game file can then refer to it.
A copy that Modn may have linked keeps the usual check: if its content
changed, recovery preserves it and stops. Runtime content in an exclusive folder, such as a log or settings
file, is part of the old release's folder: a successful update deletes it with
that folder, exactly like an uninstall. Copy such files first if you need them.
