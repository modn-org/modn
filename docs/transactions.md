# Transactions and recovery

## Ownership and recovery

The Cyberpunk Lua policy prefers an **exclusive mod folder** for new CET, RED4ext, and mod-specific redscript subdirectories when their shared parent already exists. Modn owns the entire newly created folder: uninstall quarantines and deletes it, including changed files and runtime-generated content. Do not place unrelated personal content in these folders. Pre-existing mod directories cannot be claimed wholesale; loose archives, tweaks and XML mappings use conservative file-level ownership. An exclusive claim never overwrites a pre-existing folder. Modn records the operating system's directory-object identity in its SQLite ledger (on Linux this requires a supported filesystem birth timestamp, not just a reusable inode). A transaction prepares each new folder at a same-volume staging name, persists that identity, and then activates it by rename. Recovery checks the directory object rather than a file marker, so interrupted cleanup remains restartable without hashing runtime contents.

`uninstall` and standalone archive installs require explicit `--yes` approval; catalog installs accept `--yes` or an interactive confirmation. The manager stores file hashes and ownership in SQLite in the state directory. It rejects existing destinations, linked paths, unsupported mod layouts, encrypted archives, oversized content, and attempts to uninstall modified files. Installation verifies archive content into manager-owned staging before creating game files; destination creation uses exclusive hard links rather than replacing existing files. Backups/witnesses remain until the SQLite ownership update commits. An interrupted transaction blocks further writes until `modn recover --yes` can safely roll it back or finish cleanup. If a witness or owned file was modified, recovery deliberately fails without deleting it; resolve that state manually before continuing.

The transaction executor is game-agnostic: Rust callers may submit an explicitly approved `InstallPlan` to `Store::install_operations_zip` for any real installation root. Under the exclusive store lock it independently re-inspects the archive, verifies archive hashes and all source sizes, validates destinations, collisions, executable payloads, ownership conflicts, and size limits before staging. The ownership ledger, rollback, crash recovery, and uninstall operate on generic installation roots. The CLI now resolves installation operations from Lua; no Cyberpunk-specific Rust installation planner or legacy compatibility adapter remains. ZIP and 7z extraction are supported; REDmod deployment remains unsupported. CET/RED4ext `.dll` plugin files are permitted within the recognized plugin layout, but neither Modn nor its preview executes them or certifies their compatibility; prohibited executable payloads remain blocked unless explicitly approved as an exact tool path by the caller.

Each committed install records the installation's game root in manager state (schema version 4, see [the data model](data-model.md#installation-roots-schema-version-4)). Install, uninstall, update and `modn recover` resolve the game root through that record. If the recorded root no longer exists, is a link or junction, or now resolves to a different canonical path, for example because a parent folder was moved and replaced by a junction, the operation refuses before it writes any file or state. Restore the folder at its recorded path, then run the command again. Interrupted installs recorded under schema version 3 or 4 roll back as before.

Transactions are **recoverable** rather than one single filesystem-wide atomic visibility event; during a multi-file operation, some files may briefly appear before all are committed. Hard links must be supported by the game volume. This implementation does not perform REDmod deployment, runtime/compatibility verification (only framework marker presence checks), automatic adoption, migration, or automatic recovery. Package updates use the same journals; see [Pinning and updating mods](updating.md). Always try it on a disposable game fixture before testing against a real game.

## Directories Modn creates

An install creates the missing parent directories of its files. Modn
journals each one before it creates it, with the directory's native identity
(volume and file index on Windows, device, inode and birth time on Linux).
The install's commit moves these records into a per-game ledger of
directories that Modn created. Uninstall, rollback, stack rollback and
`modn recover` remove a recorded directory deepest first, and only in these
cases:
- it is empty
- it is still a real directory, not a link
- its identity still matches

A directory that holds other content stays recorded; a later uninstall or
`modn recover` removes it once it is empty. A directory that was replaced is
forgotten and left alone. A directory that existed before Modn is never
recorded, so Modn never removes it, even when it is empty. The ledger belongs
to the game, not one package, so directories that several packages share stay
until the last file in them is gone. A record without an identity proves
nothing: a crash may have come before the `create_dir`, and another tool may
have made the directory since. Modn forgets such a record and never acts on it.
So on platforms without a native directory identity, Modn records nothing and
leaves the directories, as before. Recovery prunes only a game root that is
still a real directory at its recorded canonical path. On Linux, directory
records are case-sensitive. A directory that a rolled-back install created but
that still holds content moves into the ledger, so a later prune can remove it.
`modn game remove` ends the records for that game.

## Explicit lifecycle extensions

[Managed lifecycle](lifecycle.md) describes acknowledged adoption and approved cleanup of modified owned files.
Normal installation still rejects existing destinations. Normal uninstall retains strict integrity checks.
