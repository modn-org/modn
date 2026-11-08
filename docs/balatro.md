# Balatro

## Detection

Balatro support is Windows-only.
Modn recognizes Steam app `2379780`.
Detection requires the `Balatro.exe` marker in the game folder.
Modn reads `steam_name` from `steamapps/appmanifest_2379780.acf` and uses its `installdir` value.
If the manifest does not name the folder, Modn uses `Balatro`.
Epic Games Store and Microsoft Store installations are unsupported.

## Roots and saves

The game root is the Steam installation folder that contains `Balatro.exe`.
Balatro loads mods from `%APPDATA%\Balatro\Mods`.
That folder is outside the game root.
The API 2 game module declares it as a content root:

```json
{"content_root":{"base":"appdata","path":"Balatro/Mods"}}
```

Modn resolves the base and validates the path.
Modn records each content-root file with a root-qualified ownership key.
The missing folder is created only inside a managed transaction.
Rollback removes only folders that Modn created.

Balatro saves live in `%APPDATA%\Balatro\1`, `%APPDATA%\Balatro\2` and `%APPDATA%\Balatro\3`.
Modn never reads or writes these save folders or the game settings.
Modn does not migrate saves.

## Lovely

Lovely 0.10.0 installs `winmm.dll` beside `Balatro.exe`.
Legacy Lovely releases install `version.dll` there.
`modn --game balatro doctor --path <game> --json` reports one of three Lovely states:

- `LOVELY_MISSING`: Modn found no Lovely library.
- `LOVELY_PRESENT`: Modn found `winmm.dll`.
- `LOVELY_LEGACY`: Modn found the legacy `version.dll` library.

These states report file presence only.
They do not prove that Lovely loads or that a mod works.

## Official packages

The official `main` bucket provides two Balatro packages.

| Package | Purpose | Approval |
| --- | --- | --- |
| `main/balatro/lovely` | Installs the Lovely native injector | `--allow-native-injector` |
| `main/balatro/steamodded` | Installs Steamodded from a pinned Git commit | Normal install consent |

`main/balatro/lovely` is a native injector package.
Modn refuses to install or update it unless you pass `--allow-native-injector`.
The refusal happens before any download or game write.
Pass the flag to both the dry run and the confirmed install:

```powershell
modn --game balatro install main/balatro/lovely --allow-native-injector --dry-run --json --path <game>
modn --game balatro install main/balatro/lovely --allow-native-injector --yes --json --path <game>
```

`main/balatro/steamodded` uses a Git source pinned to one commit.
Modn fetches the pinned commit from upstream and installs the packed tree through the normal transaction.
The package declares no `sha256`; the commit is the pin.
See [Git source](sources.md#git-source).

```powershell
modn --game balatro install main/balatro/steamodded --dry-run --json --path <game>
modn --game balatro install main/balatro/steamodded --yes --json --path <game>
modn --game balatro list --json
modn --game balatro uninstall main/balatro/steamodded --yes --json
```

## Package layouts

Modn recognizes these Balatro mod layouts:

| Package content | Installation layout |
| --- | --- |
| Folder with `<id>.json` and a `main_file` entry | `<folder>/...` |
| Single `.lua` file | `<file>.lua` |
| Folder with `lovely.toml` | The folder itself |
| Steamodded tree | `smods/...` |

Modn strips one outer wrapper folder and one `Mods/` folder before it applies the layout.
A package with more than one top-level root is refused with `MULTIPLE_TOP_LEVEL_ROOTS`.
A package that contains save data is refused with `SAVE_DATA_PAYLOAD`.
A mod payload that contains `.exe`, `.bat` or `.dll` files is refused with `EXECUTABLE_PAYLOAD`.

## Limits

Modn does not edit the Balatro load order.
Modn does not migrate saves.
Modn does not validate mods at runtime.
A successful install proves the file layout and ownership record only.
It does not prove that Balatro or Steamodded loads the mod.

## Verification

Balatro support needs Modn 0.5.0 or later and `main` bucket release `v2026.11.08` or later.
Modn 0.4.0 supports Balatro but cannot verify the signed bucket releases.

Fixture tests cover detection, layouts, refusals, the native injector opt-in and the Git source.
A live run on 2026-11-07 used Modn v0.4.0 on Windows 11 with the Steam game.
It installed Lovely and Steamodded and loaded Steamodded v26.1002.0, and uninstall removed both.
The record is on [issue #52](https://github.com/modn-org/modn/issues/52).
