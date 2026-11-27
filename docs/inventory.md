# Inventory and ownership drift

`modn --game <game-id> scan --json --path <game>` combines on-disk discovery with SQLite ownership records. The `mods` array labels discovered legacy archives as `managed` only when the observed path resolves to a recorded, manager-owned file (including valid Windows case-only aliases); managed entries also include `package_id` and `integrity`. The Cyberpunk scanner emits individual CET/RED4ext files, redscript and tweak files, ArchiveXL metadata, and redscript hints for exact file-level ownership attribution; empty mod directories remain unmanaged. To preserve the one-second Lua execution budget, CET, RED4ext, redscript, hint and tweak traversal shares a bounded 256-entry budget; larger inventories return `incomplete: true` and `SCAN_INCOMPLETE` diagnostics rather than aborting. The ownership ledger still lists all recorded files. Unmanaged content and REDmod folders remain unmanaged even if they appear alongside owned files.

A module with a `content_root` makes `scan` and `doctor` read both installation roots. A `mods`, `owned_files` or `framework_signals` entry from the content root carries `"root": "content"`. Game-root entries omit `root`, so game-root-only JSON does not change. Text output prefixes content-root entries with `content`, and `scan` prints the declared content root. Lua receives both roots as numeric root IDs; it never receives an absolute content-root path. An API 1 module, or a module without a content root, uses only the game root as before.

`owned_files` reports recorded file paths, including files missing from discovery. Shared-location files use `intact`, `modified`, `missing`, `unsafe`, or `unreadable` and retain BLAKE3 integrity checks. Files under an exclusively owned directory report `present` when that real directory exists; Modn does not hash those files on scan or uninstall. Missing, linked, or unreadable owned directory roots report `missing`, `unsafe`, or `unreadable`. The original hashes remain installation evidence, not an uninstall gate for exclusive folders. `doctor` warns on missing, modified, unsafe, or unreadable managed content and reports `UNMANAGED_CONTENT` when unmanaged entries are present. Each of these diagnostics gives a next safe action in human and JSON output.

Before the first install, run `modn doctor --path <game>` as a report-only setup check. It lists detected unmanaged mod content and leftovers using the game module's scan rules. Follow the original uninstall instructions before a clean install. Modn does not remove or modify unmanaged content. An incomplete scan cannot establish that the game is clean. The check is explicit and does not run automatically during install.

For a declared content root, `doctor` reports a root that is missing, is not a folder, is a link or junction, moved from its recorded canonical path, or cannot be inspected as `CONTENT_ROOT_MISSING`, `CONTENT_ROOT_NOT_DIRECTORY`, `CONTENT_ROOT_LINKED`, `CONTENT_ROOT_MOVED` or `CONTENT_ROOT_UNAVAILABLE`. It reports recorded content files that are absent as `CONTENT_ROOT_OWNED_FILE_MISSING`. Doctor reads both roots and writes nothing.

`doctor` warns with `MODULE_SOURCE_UNTRUSTED` when the active game module comes from a bucket that is not trusted for modules, or whose signer that bucket no longer authorizes. Doctor applies the same signer rule as module loading: a release kept under a previous signer after a rotation stays authorized. A revoked signer is not authorized, unless the offline revocation key later re-authorizes that key. Then the releases the bucket retained from it are authorized again. It reports `MODULE_HIDDEN_BY_OFFICIAL` as information when a trusted catalog's module is hidden by the official module for the same game. See [game modules](game-modules.md) and [catalog key rotation and revocation](buckets.md#catalog-key-rotation-and-revocation).

The `framework_signals` array identifies candidate CET, RED4ext, redscript/compiler, ArchiveXL and TweakXL directories or binaries with their relative paths. Doctor emits `REDSCRIPT_NOT_INSTALLED`, `ARCHIVEXL_NOT_INSTALLED`, or `TWEAKXL_NOT_INSTALLED` when corresponding content is present but a known framework marker is absent. A presence signal **does not verify that a framework is installed, active, or compatible**. `scan` and `doctor` never write to the game or adopt files; the state database remains outside the game installation. A game directory redirect is not followed for ownership verification. No automatic repairs or deletion are performed.

Scan JSON now includes `scan_issues`, distinguishing entry-limit truncation
(`SCAN_INCOMPLETE`) from unsafe/linked (`SCAN_UNSAFE_ENTRY`) and unreadable
(`SCAN_UNREADABLE_ENTRY`) entries. On Windows, a verified file symlink named `ArchiveXL.log` inside
`red4ext/plugins/ArchiveXL/` is recognized as generated log output when the
ArchiveXL DLL is present, and reported informationally as `SCAN_IGNORED_GENERATED_LOG` rather than missing
mod content. Directory symlinks and special objects stay unsafe; Unix
conservatively reports an unknown link type. The link is never followed, and
skipped entries remain visible in diagnostics; scanning never grants ownership.

## Profile health

`modn doctor` reports the active profile in text and in a `profile` object under
`--json`. The object has `name`, `active`, `packages`, `overlapping_files`,
`consistent` and `issues`. `packages` counts enabled and disabled memberships.
`overlapping_files` counts paths that two or more enabled members claim. An
installation without an active profile has a null `profile`.

Each issue has `kind`, `path`, `package`, `detail` and `advice`. The kinds are:

- `winner_mismatch`: the stored winner is not the priority winner, or no winner
  row exists. The advice is `modn profile apply --yes`.
- `shadowed_claim_unrecoverable`: a shadowed claim has no verified archive, or
  the archive lacks its entry. The advice is `modn cache show`.
- `other_profile_archive_missing`: a package that another profile would write
  has no verified archive. The detail names that profile.

`consistent` is false when any issue exists. Doctor reads manager state and the
archive cache. It never repairs, never writes a game file and never changes the
cache. A missing archive therefore stays reported until you reacquire the
package into the verified cache.

`modn list` shows the active profile and a rank marker for each enabled member.
A disabled member is marked `disabled`. A package with shadowed files shows the
count. `modn list --json` stays one array. Each package adds `profile`, `rank`,
`enabled`, `shadowed_files` and `priority`. See [cli.md](cli.md#modn-profile).
