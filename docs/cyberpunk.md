# Cyberpunk 2077

## Installations

The module declares these store identities: Steam app `1091500`, GOG product
`1423049311` and Epic `AppName` `Ginger`. `modn game detect` finds the folders
that those libraries list. Registration requires `bin/x64/Cyberpunk2077.exe`
in the folder. See [cli.md](cli.md#game-detection).

## Launch

`modn play cyberpunk-2077` starts the game. When detection shows that Steam
installed the selected folder, Modn opens `steam://rungameid/1091500`. Any other
installation starts `bin/x64/Cyberpunk2077.exe` with the game folder as its
working directory. The module declares no GOG or Epic launch URI because their
formats are not verified; those installations use the executable fallback.
The `launch` target needs the Modn release that reads it; Modn 0.5.0 refuses
the newer module manifest as an unknown field.

```powershell
modn play cyberpunk-2077 --json
modn play cyberpunk-2077 --path "<game-root>"
```

## Detection

Current detection recognizes legacy `.archive` files, ArchiveXL `.xl` metadata, redscript `.reds` files, redscript hint `.toml`, tweak `.yaml`/`.yml` files, REDmod directories, CET mods and RED4ext plugins. REDmod presence means its executable exists; it is **not** a claim that deployment or game compatibility has been validated.

## Archive preview examples

```powershell
modn --game cyberpunk-2077 install "<mod-package.zip>" --dry-run --path "<game-root>" --json
modn --game cyberpunk-2077 install "<mod-package.zip>" --yes --path "<game-root>" --json
```

The first command is read-only; the second writes only after explicit approval and validation.
Both CET and RED4ext are **game policy**, not hardcoded Rust installers. A CET mod
can contain multiple Lua/config files, while RED4ext plugins can include DLLs
and nested assets. These are not claims that the required framework is present
or that a particular mod is compatible with the installed game version.
Recognized directory prefixes use canonical casing (`bin/x64/plugins/cyber_engine_tweaks/mods`,
`red4ext/plugins`, `r6/scripts`, `r6/input`, `r6/tweaks`, `r6/config/redsUserHints` and
`archive/pc/mod`); the per-mod filenames retain their original spelling.
Mismatched prefix casing, unrelated file types, REDmod, installers or unsafe ZIP
names block the **entire** package. The presence of `engine/tools/scc.exe`,
`red4ext/plugins/ArchiveXL/ArchiveXL.dll` or
`red4ext/plugins/TweakXL/TweakXL.dll` is required respectively for redscript
sources/hints, ArchiveXL metadata or TweakXL YAML tweaks. Missing markers return
specific blocked-preview codes; a marker does **not** establish a working
framework, game-version compatibility or per-mod dependency completeness.

## Supported packages

The Cyberpunk Lua policy permits ZIP and 7z packages containing recognized legacy archives, ArchiveXL metadata, redscript sources/hints, YAML tweaks, CET mods and RED4ext plugin files, alone or together, with an optional wrapper and recognized documentation. A supported multi-file package is installed and uninstalled under one ownership record. Unknown or unsupported layouts block the entire package; `mods` (REDmod), unknown framework/bootstrap files and executable installers remain out of scope. Inspect and use `install --dry-run` first. Existing manually installed files remain unmanaged; remove them before installing a conflicting package through Modn.

```powershell
modn --game cyberpunk-2077 install "C:\Downloads\HD Reworked.zip" --dry-run --json --path <game>
modn --game cyberpunk-2077 install "C:\Downloads\HD Reworked.zip" --yes --json --path <game>
modn list --json
modn uninstall <package-id> --yes --json
modn recover --yes --json
```

## Cyberpunk framework ZIP installation

CET (Cyber Engine Tweaks), RED4ext and redscript compiler distributions can
now be installed and removed through the ordinary ZIP lifecycle. The Cyberpunk
Lua policy recognizes each supported distribution through paired
bootstrap markers and mandatory ZIP members in the same wrapper. Use `install --dry-run` to
verify the whole archive, then `install --yes` to record **one package** per
framework ZIP. Partial and mixed framework distributions remain blocked.

Framework payloads use **file-level ownership** even inside CET and RED4ext
directories: other mods, personal configuration and generated runtime files
remain separate. Uninstall verifies hashes of owned framework files and
leaves unowned logs and settings untouched; modified bootstrap files block
uninstall. Rust denies executable payloads by default, including the redscript compiler.
Installing redscript requires the operator to approve `engine/tools/scc.exe`:
at the catalog install prompt, or by adding
`--allow-executable engine/tools/scc.exe` to **both** the review preview and
confirmed installation. This is a trusted, explicit approval of one exact
game-relative tool executable, independent of Lua operation labels.
Without that specific approval, executables remain prohibited. Explicit tool-executable permission does not certify the executable as trustworthy.
Modn does not execute framework binaries or certify runtime compatibility
or game-version support. Only catalog-installed dependents block a framework
uninstall; standalone archives that need a framework are not tracked.

## Common Cyberpunk prerequisite package layouts

The following is a **file-layout installation** compatibility list, not a
claim that Modn validates game-version compatibility or mod dependencies.
All eleven original local distributions were installed and uninstalled together
in an isolated game fixture:

| Package | Recognized installation layout | Archive exercise |
| --- | --- | --- |
| ArchiveXL | RED4ext plugin and redscript hint | Original ZIP |
| Browser Extension | `r6/scripts/BrowserExtension/*.reds` | Original solid 7z |
| Codeware | RED4ext plugin | Original ZIP |
| Cyber Engine Tweaks | CET framework bootstrap and assets | Original ZIP |
| Equipment-EX | ArchiveXL metadata, redscript and redscript hint | Original ZIP (five payload files; LICENSE skipped) |
| Mod Settings | RED4ext plugin | Original ZIP |
| No Intro Videos | redscript source | Original ZIP |
| RED4ext | Framework bootstrap | Original ZIP |
| TweakXL | RED4ext plugin | Original ZIP |
| Virtual Atelier | ArchiveXL metadata, archives, redscript and hint | Original ZIP |
| redscript | Compiler and framework configuration | Original ZIP |

The redscript framework ZIP additionally requires explicit approval of
`engine/tools/scc.exe`, either at the catalog install prompt or with
`--allow-executable engine/tools/scc.exe` on both preview and installation. Third-party mod ZIPs/7z files cannot include unapproved
executable payloads. The game-agnostic transaction engine streams 7z
members directly into private staging with the same archive hash, file-size,
ownership and crash recovery checks as ZIP; it never extracts unselected
content into the game. Lua's read-only archive inspection capability still excludes 7z.
