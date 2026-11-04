# Managed content lifecycle foundations

These APIs extend Rust ownership. They do not authorize deleting arbitrary paths.
All examples must use disposable fixtures until the CLI integration is verified.

## REDmod content

The Cyberpunk Lua policy recognizes one canonical `mods/<name>/info.json` package.
It retains `<name>` and accepts associated files under `archives`, `scripts`, and `tweaks`.
It rejects mixed layouts, multiple names, noncanonical directory casing, and occupied package directories.
The metadata preview does not extract or validate file contents.

The Lua proposal marks `info.json` with the generic `json-object` content kind.
Rust validates that bounded payload after extraction into manager staging, before any game write.
The payload must contain a JSON object and use at most 65,536 bytes.
This check does not validate the complete REDmod vendor schema.
Other games can use the same content kind without a Rust game adapter.

Use `planning::preview_install_zip` and `Store::install_operations_zip` through the normal install path.
Use `inventory::scan_with_ownership_for` and `inventory::doctor_with_ownership_for` for managed inventory and diagnostics.
The Lua scanner retains directory-only REDmod detection. Rust reports individual claims in `owned_files` with package IDs and integrity status.
The directory detection remains unmanaged because individual file claims do not confer directory ownership.
Use `Store::uninstall` for normal removal.

The first MVP does not invoke REDmod deployment or undeployment.
It does not verify game compatibility, vendor schema semantics, generated deployment output, or activation state.
Tool presence remains a diagnostic hint. Content installation does not require the deployment executable.

## Explicit manual adoption

Call `planning::preview_adoption(game_id, game, archive)` for a metadata preview.
Show the selected files and the archive SHA-256 before requesting acknowledgement.
The preview permits existing targets. It grants no write authority and does not verify target contents.

Call this API after explicit acknowledgement:

```text
Store::adopt_archive(
    game_id, game, archive, reviewed_sha256,
    transactions::AdoptionApproval::Acknowledged,
)
```

`AdoptionApproval::Denied` refuses without recording ownership.
The API regenerates the current module proposal under the manager lock.
It rejects changed archives, blocked proposals, missing files, unsafe paths, ownership collisions, and recorded conflicts.
Every selected target must match its archive member by byte size and SHA-256.
Rust records individual file claims and archive provenance in one SQLite transaction.
It creates no game file, directory claim, or claim over unknown descendants.
Normal install continues to reject existing files.

Adoption currently uses local ZIP or 7z archives without catalog identity or dependency registration.
Archive SHA-256 identifies provenance. The schema does not distinguish adopted packages from installed packages.
Tool-executable permission grants are unavailable through the adoption API.
The existing filesystem race limitations still apply when another process changes game paths concurrently.

## Approved cleanup of modified owned files

Call `Store::cleanup_owned(game, package_id, transactions::CleanupApproval::Approved)` after explicit approval.
`CleanupApproval::Denied` refuses without changing game bytes or ownership.
The API selects a managed package ID within the selected installation.
It accepts no raw-path removal list.

Cleanup removes recorded files individually, including modified files inside an exclusive directory.
It preserves unrecorded descendants, other managed packages, and unsafe or replaced directories.
It rejects missing owned files and packages required by another package.
It retains exclusive directories. Empty manager-created parents use the existing pruning rules.
Normal `Store::uninstall` retains its strict checks.

Cleanup journals current file digests and retains hard-link backups until the ownership commit.
Automatic rollback restores current bytes on failure.
`Store::recover` restores interrupted cleanup using the same witnesses.
Recovery verifies recorded exclusive-directory identities before restoring files.
Changed paths or witnesses stop recovery and preserve the remaining evidence.

## Intended CLI integration

The integration worker owns registration. These commands are not registered by this change:

```text
modn adopt <archive> --game <game-id> --path <game> --dry-run --json
modn adopt <archive> --game <game-id> --path <game> --acknowledge --yes
modn uninstall <package-id> --path <game> --force --yes
```

Resolve one installation through the existing selection API before either action.
Adoption dry runs call `preview_adoption`. Execution passes the reviewed archive digest to `adopt_archive`.
The CLI must require both explicit acknowledgement and mutation approval for adoption.
Force cleanup requires `--force` and mutation approval, then calls `cleanup_owned`.
Without `--force`, keep the normal uninstall path.
Never infer either approval from Lua labels, archive contents, scan results, or unmanaged detection.

## Verification

`transactions::lifecycle_tests` exercises real Store, archive, path, and file state.
It covers REDmod lifecycle and refusal, atomic adoption failure, authorization, modified cleanup, and recovery.
Its fixed-seed property checks 32 generated payload and approval cases.
Existing destination properties and CLI ownership properties retain their original assertions.
