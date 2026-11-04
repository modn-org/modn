# Trusted game modules and Skyrim Special Edition

This MVP adds core APIs. CLI command registration is an integration task.
Third-party modules use a separate explicit registry. Community buckets still cannot publish modules.
Trust acknowledgement accepts executable Lua policy. Do not describe this as sandboxing.
Registration, listing, and removal change no game files.
Registration does not approve installation or executable payloads.

## API wiring

Use `modn_core::data_dir::data_directory()` for the selected manager data directory.
Construct `game_modules::ModuleRegistry::new(&data_directory)`.

| Future command | Exact API | Result |
| --- | --- | --- |
| `modn module register <source> --trust` | `registry.register(source, ModuleTrust::Acknowledged)` | `RegisteredModule` |
| Registration without `--trust` | `registry.register(source, ModuleTrust::NotAcknowledged)` | Explicit trust error |
| `modn module list` | `registry.list()` | Sorted `Vec<RegisteredModule>` |
| `modn module remove <game-id>` | `registry.unregister(game_id)` | Removes only the copied snapshot |
| `modn module check <source>` | Existing `game_modules::check_module(source)` | Read-only identity validation |

Show `module.game_id`, `module.version`, `module.api_version`, and `sha256` in list and registration output.
The digest covers the exact manifest bytes followed by the entrypoint bytes.
The snapshot resides at `<data-directory>/game-modules/<game-id>/`.
It contains `module.json`, the single Lua entrypoint, and `registration.json`.
Other source files are not copied. Source deletion does not invalidate a registered snapshot.
Registration validates the copied Lua identity before publishing the snapshot.
Registry mutations hold an exclusive file lock. Publication uses a directory rename.
Interrupted staging directories remain inactive. This MVP does not provide staging cleanup or power-loss durability guarantees.
Existing registrations require explicit removal before replacement.
Corrupt registrations can be removed without executing their Lua source.
Removal leaves installed package records and ownership intact. Managed uninstall uses those records without the module.

Registration and normal resolution require a valid local `main` bucket.
Run `modn bucket update` first. The registry never fetches it implicitly.
Registration refuses game IDs published by the validated official bucket.
Normal resolution chooses official policy for official IDs and registered policy for other IDs.
Explicit `with_module_source(Some(source), action)` retains highest priority.
The existing `MODN_GAME_MODULES_DIR` override retains priority over persistent sources.
Missing, incompatible, or mismatched overrides fail without fallback.
Snapshot reads reject changed bytes or identity. Digests detect changes; they do not prove publisher authenticity.
Concurrent external filesystem mutation is outside the snapshot guarantee.

CLI game selection must pass the selected runtime game ID to existing core operations.
Do not introduce a Skyrim Rust adapter or default game ID.
The relevant entry points are:

- `discovery::validate_installation_for(game_id, path)`
- `planning::preview_install_zip(game_id, path, archive)`
- `Store::install_operations_zip(path, archive, &preview)` after existing explicit approval checks
- `inventory::scan_with_ownership_for(&store, game_id, path)`
- `inventory::doctor_with_ownership_for(&store, game_id, path)`
- `Store::uninstall(path, package_id)` after existing explicit approval checks

Integration must add registered IDs to any CLI supported-game listing that currently lists only official IDs.
The core resolver already selects registered modules by the runtime game ID.
No root CLI wiring, bucket relocation, or root documentation changes belong to this implementation.

## Skyrim example

Source: `examples/game-modules/skyrim-special-edition/`.
Register that directory with explicit trust, or select it through the existing source override.
Select game ID `skyrim-special-edition` and an explicit disposable or user-selected game path.
The module requires a regular `SkyrimSE.exe` file and a real `Data` directory.
It does not infer game identity from a folder name. It supplies no automatic discovery paths.

Archives must use exact `Data/` prefixes. Wrapped archives and bare plugin files are rejected.
Supported files are top-level `.esp`, `.esm`, `.esl`, and `.bsa` files under `Data`.
Loose files are limited to `.nif` under `Data/meshes/` and `.dds` under `Data/textures/`.
Filename extension checks ignore case. Directory names retain their supplied spelling.
Root `readme.txt` and `license.txt` are ignored. Other unsupported files block the plan.
All supported files use individual ownership in shared locations.
Rust retains destination validation, overwrite protection, rollback, recovery, and uninstall authority.

Scan reports candidate Data content. It does not distinguish official files from manually installed content.
Doctor reports scan limits and managed-file drift. It always reminds the user to manage load order.
No SKSE, DLLs, Papyrus scripts, FOMOD installers, save migration, automatic activation, or load-order editing is supported.
The module does not validate plugin dependencies, game versions, BSA contents, or in-game behavior.

## Evidence and limits

`cargo test -p modn-core --locked --test module_registry --test skyrim_module`
uses copied modules, a seeded disposable official bucket, a synthetic executable marker, and a ZIP fixture.
The lifecycle covers validation, read-only preview, managed installation, inventory, doctor, registration removal, and managed uninstall.
It verifies that manual Data content and `plugins.txt` retain their original bytes.
Negative cases cover missing trust, official collisions, incompatible API, mismatched identity, duplicate registration, and changed snapshot bytes.
Explicit missing and mismatched overrides fail even when a valid registration exists.
These tests do not run Skyrim. No real game operation has been performed.
