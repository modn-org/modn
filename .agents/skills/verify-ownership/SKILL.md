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
| `dry_run_install_and_uninstall_keep_ownership_promises` | A dry run changes no file and no manager state. An install writes exactly the previewed files or changes nothing. Overlapping installs keep exactly one winner per path, and uninstalling every package in any order restores the game tree. A profile switch writes exactly the previewed files and leaves unmanaged files alone. Profile disable and enable, priority apply and switch round trips restore the tree. | Previews write, installs overwrite user files or write outside the game, a winner, uninstall or switch deletes or keeps another package's or the user's files, or a round trip changes a byte it did not name. |
| `reviewable_dry_run_predicts_the_install` | When nothing changes between them, a `reviewable` dry run and the approved install agree. | The user approves a preview, and the install then fails on a blocker the preview missed. |

Each check prints how many items it checked, such as
`containment: 235 accepted destinations checked`. A run that checks fewer
than its floor fails, so a vacuous pass cannot happen.

## Coverage record

Last confirmed against commit `168e12e` after Milestone 20.

| Check | Covered code |
|---|---|
| containment, documented names, superscript device names, single component | `crates/modn-core/src/archive.rs` `validated_name`; the destination gate in `crates/modn-core/src/planning.rs` `resolve_with_permissions`, `crates/modn-core/src/planning/destination.rs` `existing_destination_issue` and `crates/modn-core/src/transactions/validation.rs` `validate_plan` (called by `validate_execution_plan` and `transactions/adoption.rs`); `crates/modn-core/src/transactions/journal.rs` `target_path`. The test function `accepted` in `crates/modn-core/src/planning/destination_properties.rs` calls `validated_name` directly, so the name rules have one owner. The single-component checks live in `destination_properties/single_component.rs`. |
| ownership key | `transactions/directory.rs` `ownership_key`, `case_key` and `directory_candidate_key`; `planning.rs` `insert_destination` (one key per root); `transactions/commit.rs` `commit_install` (one `managed_files` winner row per `(game_path, root, path_key)` with its `profile_id`, one `file_claims` row per claiming package, and refusal or override of another package's winner) and `commit_profile_switch` (old winner rows deleted, target winners inserted); `transactions/claim.rs` `managed_claim_keys`, `file_reservations`, `shadowed_claim_path`, `shadowing_claim_path` and `claims_in_active_profile`; `transactions/restore.rs` `next_claim` (highest-priority enabled claimant of the active profile); `crates/modn-core/src/profiles.rs` `create`, `set_priority` and `set_enabled`; `crates/modn-core/src/state/profiles.rs` `ensure_profile`, `profile_id`, `append_package` and `delete_package_records`; `transactions/journal.rs` `Roots` and `target_path`; `transactions/created_dirs.rs`; `transactions/validation.rs` `validate_ownership_paths` and `Store::transaction_root`; `state.rs` `migrate_ownership_keys` and its migration and collision tests in `state/tests.rs`; `state/roots.rs` `SCHEMA_VERSION` (8), `record_root`, `check_root`, `recorded_root` and `resolve_root`, `RootKind::Content`, and `crates/modn-core/src/content_root.rs` resolution, whose root-qualified keys `(game_path, root, path_key)` the checks reach through `commit_install`, `validate_ownership_paths` and `Store::transaction_root`, with tests in `state/roots_tests.rs` and `transactions/roots_tests.rs`. |
| end to end, agreement | `apps/modn-cli/src/install.rs` `install_archive`; `apps/modn-cli/src/managed.rs` `uninstall`; `apps/modn-cli/src/profile.rs` `open_installation` and `apps/modn-cli/src/profile/{switch,enable,priority,lifecycle}.rs` (the generators run create, switch, enable, disable and priority; `profile apply`, `profile list` and `profile delete` are not run); `apps/modn-cli/src/conflicts.rs`; `apps/modn-cli/src/state_access.rs`; `crates/modn-core/src/planning/preview.rs` `preview_install_zip_with_claims`, `preview_install_zip_with_permissions` and `preview_install_zip_with_appdata_permissions`; `planning.rs` `resolve_with_permissions`; `crates/modn-core/src/transactions/` `install/execute.rs` (`install_locked`), `uninstall.rs`, `uninstall_plan.rs` (`uninstall_with`), `restore.rs` (`next_claim`, `stage_restores`), `restore_recovery.rs`, `override_recovery.rs`, `recovery.rs`, `commit.rs`, `claim.rs`, `validation.rs`, `journal.rs`, `directory_ledger.rs` (`installed`), `created_dirs.rs`, `profile_toggle.rs` with `profile_toggle_plan.rs` and `profile_toggle_apply.rs`, `profile_apply.rs` (`plan_priority`, `set_priority_applied`) with `profile_apply_plan.rs`, and `profile_switch/` (`plan.rs`, `execute.rs`, `refuse.rs`, `recover.rs`); `crates/modn-core/src/profiles.rs`; `crates/modn-core/src/conflicts.rs` `conflicts`; `crates/modn-core/src/state/profiles.rs`. |

The checks do not reach these paths today:

- A catalog install or update. `crates/modn-core/src/catalog_pipeline/` (`commit_plan` and `commit_update`) commits through `transactions/install/planned.rs` (`install_catalog_release_planned`), `transactions/stack.rs` (`install_catalog_stack`) and `transactions/update.rs` (`update_catalog_release_planned`), and its `commit` now reaches the override path. `apps/modn-cli/src/catalog_install.rs` runs the catalog install command, including its dry run. `catalog_pipeline/tests/override_tests.rs` covers an override by example, but no property generator runs a catalog install.
- A declared content root. The CLI generator rewrites the Cyberpunk policy without `content_root`, so every operation uses the `game` root. The ownership-key property also tests only the `game` root. `apps/modn-cli/tests/content_root_cli.rs` covers a content-root install, dry run, uninstall and update by example.
- Scan and doctor reconciliation. `crates/modn-core/src/inventory.rs` and `crates/modn-core/src/inventory/profile_health.rs` `health` read the same ownership keys, claims and winners against the disk, but no property runs a scan or doctor report. `inventory/profile_health_tests.rs` and the CLI example tests cover them.
- Profile management outside the modeled path. The generators run `profile create`, `switch`, `enable`, `disable`, `priority --apply` and `conflicts` on the active profile. They do not run `profile delete`, `profile create --clone`, `profile apply --yes`, `profile list`, `conflicts --profile` on an inactive profile, or a crash at a transaction point. The unit and crash tests under `crates/modn-core/src/` (`profiles/tests.rs`, `profile_apply_tests.rs`, `conflicts/tests.rs` and the `*_crash_tests.rs` files) cover those by example.

To see whether the covered code changed after the sync commit, run:

```sh
git diff --stat 168e12e -- crates/modn-core/src/archive.rs crates/modn-core/src/planning crates/modn-core/src/state.rs crates/modn-core/src/state/roots.rs crates/modn-core/src/state/profiles.rs crates/modn-core/src/content_root.rs crates/modn-core/src/inventory.rs crates/modn-core/src/inventory crates/modn-core/src/profiles.rs crates/modn-core/src/conflicts.rs crates/modn-core/src/transactions crates/modn-core/src/catalog_pipeline apps/modn-cli/src/install.rs apps/modn-cli/src/managed.rs apps/modn-cli/src/state_access.rs apps/modn-cli/src/catalog_install.rs apps/modn-cli/src/catalog_install apps/modn-cli/src/profile.rs apps/modn-cli/src/profile apps/modn-cli/src/conflicts.rs
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

The first command takes about a second and the second a few minutes. The
seed is fixed, so a rerun checks the same cases. To explore new cases, set
`PROPTEST_RNG_SEED` to any integer. To check more cases, set `PROPTEST_CASES`.
`cargo test --workspace` runs all seven checks with their default case counts.

The ownership-key property is meaningful on Windows, where NTFS folds case.
On a case-sensitive file system it finds no same-file pairs, and it then skips
its floor.

## Expected result at the sync commit

All seven checks pass. The core run prints `containment`, `documented names`,
`superscript device names`, `ownership key` and `single component` counts. The
first four have a floor of 50 accepted destinations; the ownership-key floor
applies on Windows. `single component` has floors of 100 accepted, 100
rejected and 20 same-file spellings. The CLI run prints the approved-install
counts with floors of 5 installs for `ownership`, 3 profile switch round trips
and 5 installs for `preview agreement`.

Destinations may have one component; `validated_name` alone decides which names are safe. File and directory ownership keys use `ownership_key`:
per-character lowercase on Windows and exact paths elsewhere. Created-directory
keys use `case_key`: per-character uppercase on Windows and exact paths elsewhere.
Writable state opens migrate stored file, directory and created-directory
keys in one transaction. Exact keys require no updates.
A collision refuses the open and
preserves every ownership record. Read-only opens do not migrate state.
Schema version 8 records each installation root in `installation_roots`, and
every ownership and created-directory key is qualified by
`(game_path, root, path_key)`. The same schema records `profiles`,
`profile_packages` and `file_claims`, and `managed_files` holds one winner row
per path with its `profile_id`. The checks record only the `game` root; a
declared content root is exercised by the content-root CLI tests, not by these
properties.

## Read a failure

`proptest` prints the failed assertion and a `minimal counterexample`. Retell
it as the shortest sequence of steps in Modn's terms: what the module proposed,
what the dry run reported, what `install --yes` or `uninstall --yes` did to
which path. Then decide which of these it is:

- **Real bug.** A property assertion failed, such as `dry run of a changed manager state`, `uninstall of ... removed the wrong files`, `switching back ... did not restore the game` or a containment mismatch. Report it with the steps. Keep the property and the generators unchanged, and change product code only when the user asks.
- **Modeling error.** The test no longer describes the code. Examples: `accepted` no longer matches the gate in `planning.rs` and `validation.rs`, or the bundled `main.lua` no longer ends in `return policy`. Fix the test so that it describes the code again.
- **Setup noise.** The failure is an I/O error from building the fixture, such as a path that is too long or a file locked by antivirus. Rerun it, and if it repeats, fix the fixture.

## Change the verification

When a change to behavior is intended, update the property sentence in the
table first, then the assertion. When new hostile input shapes appear, add them
to the generator pools `HOSTILE`, `NAMES`, `SHARED`, `SINGLE_PREFIXES`,
`SINGLE_SUFFIXES`, `SINGLE_DEVICES`, or the CLI pools `SHARED` and `HOSTILE`.
Show every changed check failing on a scratch break of the code it guards,
such as removing the trailing-dot or `CON` rule in `validated_name`, recording
ownership for only the first file in `commit_install`, making the dry run
open writable state, or skipping one write in
`transactions/profile_switch/execute.rs` `place`. Each of these was caught at
the sync commit, except the switch break, which was caught when the switch
property was added. Then revert the break and update the coverage record
and the sync commit.
