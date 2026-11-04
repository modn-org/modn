# Inventory and ownership drift

`modn --game <game-id> scan --json --path <game>` combines on-disk discovery with SQLite ownership records. The `mods` array labels discovered legacy archives as `managed` only when the observed path resolves to a recorded, manager-owned file (including valid Windows case-only aliases); managed entries also include `package_id` and `integrity`. The Cyberpunk scanner emits individual CET/RED4ext files, redscript and tweak files, ArchiveXL metadata, and redscript hints for exact file-level ownership attribution; empty mod directories remain unmanaged. To preserve the one-second Lua execution budget, CET, RED4ext, redscript, hint and tweak traversal shares a bounded 256-entry budget; larger inventories return `incomplete: true` and `SCAN_INCOMPLETE` diagnostics rather than aborting. The ownership ledger still lists all recorded files. Unmanaged content and REDmod folders remain unmanaged even if they appear alongside owned files.

`owned_files` reports recorded file paths, including files missing from discovery. Shared-location files use `intact`, `modified`, `missing`, `unsafe`, or `unreadable` and retain SHA-256 integrity checks. Files under an exclusively owned directory report `present` when that real directory exists; Modn does not hash those files on scan or uninstall. Missing, linked, or unreadable owned directory roots report `missing`, `unsafe`, or `unreadable`. The original hashes remain installation evidence, not an uninstall gate for exclusive folders. `doctor` warns on missing, modified, unsafe, or unreadable managed content and reports `UNMANAGED_CONTENT` when unmanaged entries are present. Each of these diagnostics gives a next safe action in human and JSON output.

Before the first install, run `modn doctor --path <game>` as a report-only setup check. It lists detected unmanaged mod content and leftovers using the game module's scan rules. Follow the original uninstall instructions before a clean install. Modn does not remove or modify unmanaged content. An incomplete scan cannot establish that the game is clean. The check is explicit and does not run automatically during install.

The `framework_signals` array identifies candidate CET, RED4ext, redscript/compiler, ArchiveXL and TweakXL directories or binaries with their relative paths. Doctor emits `REDSCRIPT_NOT_INSTALLED`, `ARCHIVEXL_NOT_INSTALLED`, or `TWEAKXL_NOT_INSTALLED` when corresponding content is present but a known framework marker is absent. A presence signal **does not verify that a framework is installed, active, or compatible**. `scan` and `doctor` never write to the game or adopt files; the state database remains outside the game installation. A game directory redirect is not followed for ownership verification. No automatic repairs or deletion are performed.

Scan JSON now includes `scan_issues`, distinguishing entry-limit truncation
(`SCAN_INCOMPLETE`) from unsafe/linked (`SCAN_UNSAFE_ENTRY`) and unreadable
(`SCAN_UNREADABLE_ENTRY`) entries. On Windows, a verified file symlink named `ArchiveXL.log` inside
`red4ext/plugins/ArchiveXL/` is recognized as generated log output when the
ArchiveXL DLL is present, and reported informationally as `SCAN_IGNORED_GENERATED_LOG` rather than missing
mod content. Directory symlinks and special objects stay unsafe; Unix
conservatively reports an unknown link type. The link is never followed, and
skipped entries remain visible in diagnostics; scanning never grants ownership.
