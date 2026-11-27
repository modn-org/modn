---
name: verify-ownership
description: Property-check Modn's path-safety, ownership and dry-run promises against generated hostile Lua proposals. Use when a change touches destination validation, ownership keys, install, uninstall or `install --dry-run`, or when a check below fails.
---

# Verify ownership and dry-run promises

The checks are `proptest` properties on the real code. The core properties
call the real validation functions. The CLI properties run the real `modn`
binary. The operating system is the oracle in both: each generated path is
created in a scratch folder, and the checks compare what the disk holds.

Windows 11 creates ordinary files named `CON` or `COM¹` inside a folder, so the
live OS cannot show the device-name harm that older Windows versions and the
Windows shell still have. The two documented-name checks use Microsoft's
[Naming Files](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file)
rules as the oracle for that class.

## Promises checked

| Check | Promise, in one sentence | Harm when broken |
|---|---|---|
| `accepted_destinations_stay_inside_the_game` | Every destination the gate accepts names one new file inside the installation root, under the same name, after the OS resolves it. | A hostile module writes outside the installation root, to a device such as `CON`, or under a silently changed name. |
| `accepted_destinations_follow_documented_windows_names` | No accepted path component is `CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9` or `LPT1`–`LPT9` (alone or before an extension), ends in a space or period, or holds a reserved character. | A package installs a file that Windows tools open as a device or cannot delete. |
| `accepted_destinations_avoid_superscript_device_names` | No accepted path component is `COM¹`–`COM³` or `LPT¹`–`LPT³`, alone or before an extension. | The same device-name harm through the superscript digits that Microsoft documents as reserved. |
| `single_component_destinations_keep_every_rule` | A destination with one component is accepted only when it names one file directly in the installation root, follows the documented Windows names, and shares its ownership key with every same-file spelling; drive, UNC, traversal, stream, trailing-dot, trailing-space and device-name forms stay rejected. | A single-file mod writes outside the installation root, to a device, or beside another package's file under a different case. |
| `same_file_spellings_share_one_ownership_key` | Two accepted destinations that the OS treats as one file get the same ownership key. | Two operations or two packages own one file, so the plan and the ownership checks miss the collision. |
| `dry_run_install_and_uninstall_keep_ownership_promises` | A dry run changes no file and no manager state. An install writes exactly the previewed files or changes nothing. An uninstall removes exactly its package's files. Uninstalling every package restores the game tree. | Previews write, installs overwrite user files or write outside the game, or uninstall deletes another package's or the user's files. |
| `reviewable_dry_run_predicts_the_install` | When nothing changes between them, a `reviewable` dry run and the approved install agree. | The user approves a preview, and the install then fails on a blocker the preview missed. |

Each check prints how many items it checked, such as
`containment: 235 accepted destinations checked`. A run that checks fewer
than its floor fails, so a vacuous pass cannot happen.

## Coverage record

Last confirmed against commit `f586621` after Milestone 17.

| Check | Covered code |
|---|---|
| containment, documented names, superscript device names, single component | `crates/modn-core/src/archive.rs` `validated_name`; the destination gate in `crates/modn-core/src/planning.rs` `resolve_with_permissions` and `crates/modn-core/src/transactions/validation.rs` `validate_plan` (called by `validate_execution_plan` and `transactions/adoption.rs`); `crates/modn-core/src/transactions/journal.rs` `target_path`. The test function `accepted` in `crates/modn-core/src/planning/destination_properties.rs` calls `validated_name` directly, so the name rules have one owner. The single-component checks live in `destination_properties/single_component.rs`. |
| ownership key | `transactions/directory.rs` `ownership_key`, `case_key` and `directory_candidate_key`; `planning.rs` `insert_destination` (one key per root); `transactions/journal.rs` `commit_install` (`path_key` and root label) and `Roots`; `transactions/created_dirs.rs`; `transactions/validation.rs` `validate_ownership_paths` and `Store::transaction_root`; `state.rs` `migrate_ownership_keys` and its migration and collision tests in `state/tests.rs`; `state/roots.rs` `SCHEMA_VERSION`, `record_root`, `check_root`, `recorded_root` and `resolve_root`, `RootKind::Content`, and `crates/modn-core/src/content_root.rs` resolution, whose root-qualified keys `(game_path, root, path_key)` the checks reach through `commit_install`, `validate_ownership_paths` and `Store::transaction_root`, with tests in `state/roots_tests.rs` and `transactions/roots_tests.rs`. |
| end to end, agreement | `apps/modn-cli/src/install.rs` `install_archive`; `apps/modn-cli/src/managed.rs` `uninstall`; `apps/modn-cli/src/state_access.rs`; `crates/modn-core/src/planning/preview.rs` `preview_install_zip_with_permissions` and `preview_install_zip_with_appdata_permissions`; `planning.rs` `resolve_with_permissions`, `existing_destination_issue`; `crates/modn-core/src/transactions/` `install.rs` (`install_operations_zip_with_permissions`), `uninstall.rs`, `uninstall_plan.rs` (`uninstall_with`), `directory_ledger.rs` (`installed`), `journal.rs`, `recovery.rs`, `created_dirs.rs`, `validation.rs`. |

The checks do not reach these paths today:

- A catalog install or update. `crates/modn-core/src/catalog_pipeline/` (`commit_plan` and `commit_update`) commits through `transactions/install/planned.rs` (`install_catalog_release_planned`), `transactions/stack.rs` (`install_catalog_stack`) and `transactions/update.rs` (`update_catalog_release_planned`), and `apps/modn-cli/src/catalog_install.rs` runs the catalog install command, including its dry run. These reach `validate_plan`, `validate_ownership` and `commit_install`, but no property generator runs them.
- A declared content root. The CLI generator rewrites the Cyberpunk policy without `content_root`, so every operation uses the `game` root. The ownership-key property also tests only the `game` root. `apps/modn-cli/tests/content_root_cli.rs` covers a content-root install, dry run, uninstall and update by example.
- Scan and doctor reconciliation. `crates/modn-core/src/inventory.rs` reads the same ownership keys against the disk, but no property runs a scan or doctor report.

To see whether the covered code changed after the sync commit, run:

```sh
git diff --stat f586621 -- crates/modn-core/src/archive.rs crates/modn-core/src/planning crates/modn-core/src/state.rs crates/modn-core/src/state/roots.rs crates/modn-core/src/content_root.rs crates/modn-core/src/inventory.rs crates/modn-core/src/transactions crates/modn-core/src/catalog_pipeline apps/modn-cli/src/install.rs apps/modn-cli/src/managed.rs apps/modn-cli/src/state_access.rs apps/modn-cli/src/catalog_install.rs apps/modn-cli/src/catalog_install
```

When that diff is not empty, read it against the table and the generators.
Move the sync commit forward only after you confirm that each check still
describes the code at the new commit.

## Run

From the repository root:

```sh
cargo test -p modn-core --locked --lib destination_properties -- --include-ignored --nocapture
cargo test -p modn-cli --locked --test ownership_properties -- --include-ignored --nocapture
```

The first command takes about a second and the second about 30 seconds. The
seed is fixed, so a rerun checks the same cases. To explore new cases, set
`PROPTEST_RNG_SEED` to any integer. To check more cases, set `PROPTEST_CASES`.
`cargo test --workspace` runs all seven checks with their default case counts.

The ownership-key property is meaningful on Windows, where NTFS folds case.
On a case-sensitive file system it finds no same-file pairs, and it then skips
its floor.

## Expected result at the sync commit

All seven checks pass. Destinations may have one component; `validated_name` alone decides which names are safe. File and directory ownership keys use `ownership_key`:
per-character lowercase on Windows and exact paths elsewhere. Created-directory
keys use `case_key`: per-character uppercase on Windows and exact paths elsewhere.
Writable state opens migrate stored file, directory and created-directory
keys in one transaction. Exact keys require no updates.
A collision refuses the open and
preserves every ownership record. Read-only opens do not migrate state.
Schema version 7 records each installation root in `installation_roots`, and
every ownership and created-directory key is qualified by
`(game_path, root, path_key)`. The checks record only the `game` root; a
declared content root is exercised by the content-root CLI tests, not by these
properties.

## Read a failure

`proptest` prints the failed assertion and a `minimal counterexample`. Retell
it as the shortest sequence of steps in Modn's terms: what the module proposed,
what the dry run reported, what `install --yes` or `uninstall --yes` did to
which path. Then decide which of these it is:

- **Real bug.** A property assertion failed, such as `dry run of a changed manager state`, `uninstall of ... removed the wrong files` or a containment mismatch. Report it with the steps. Keep the property and the generators unchanged, and change product code only when the user asks.
- **Modeling error.** The test no longer describes the code. Examples: `accepted` no longer matches the gate in `planning.rs` and `validation.rs`, or the bundled `main.lua` no longer ends in `return policy`. Fix the test so that it describes the code again.
- **Setup noise.** The failure is an I/O error from building the fixture, such as a path that is too long or a file locked by antivirus. Rerun it, and if it repeats, fix the fixture.

## Change the verification

When a change to behavior is intended, update the property sentence in the
table first, then the assertion. When new hostile input shapes appear, add them
to the generator pools `HOSTILE`, `NAMES`, `SHARED`, `SINGLE_PREFIXES`,
`SINGLE_SUFFIXES`, `SINGLE_DEVICES`, or the CLI pools `SHARED` and `HOSTILE`.
Show every changed check failing on a scratch break of the code it guards,
such as removing the trailing-dot or `CON` rule in `validated_name`, recording
ownership for only the first file in `commit_install`, or making the dry run
open writable state. Each of these was caught at the sync commit. Then revert the break and update the coverage record
and the sync commit.
