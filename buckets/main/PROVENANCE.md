# Official `main` bucket provenance

Evidence for each Cyberpunk 2077 package in the bundled snapshot, inspected
2026-10-31. Modn does not mirror or redistribute any of these archives.

## Sources

- **https**: the original GitHub release asset. Its SHA-256 matches the
  `digest` GitHub publishes for that asset and a fresh download.
- **manual**: Nexus Mods only. Nexus requires a signed-in download, and its
  authors have not granted Modn permission to redistribute. Users download
  the named file themselves; the hash was taken from that original download.

| Package | Version | Source | Upstream |
| --- | --- | --- | --- |
| `red4ext` | 1.31.0 | https | [WopsS/RED4ext](https://github.com/WopsS/RED4ext) |
| `redscript` | 0.5.32 | https | [jac3km4/redscript](https://github.com/jac3km4/redscript) |
| `cet` | 1.38.1 | https | [maximegmd/CyberEngineTweaks](https://github.com/maximegmd/CyberEngineTweaks) |
| `archivexl` | 1.28.6 | https | [psiberx/cp2077-archive-xl](https://github.com/psiberx/cp2077-archive-xl) |
| `tweakxl` | 1.12.0 | https | [psiberx/cp2077-tweak-xl](https://github.com/psiberx/cp2077-tweak-xl) |
| `codeware` | 1.21.2 | https | [psiberx/cp2077-codeware](https://github.com/psiberx/cp2077-codeware) |
| `mod-settings` | 0.2.28 | https | [jackhumbert/mod_settings](https://github.com/jackhumbert/mod_settings) |
| `input-loader` | 0.2.6 | https | [jackhumbert/cyberpunk2077-input-loader](https://github.com/jackhumbert/cyberpunk2077-input-loader) |
| `better-loot-markers` | 1.4.1 | https | [rodikh/BetterLootMarkers-Cyberpunk-mod](https://github.com/rodikh/BetterLootMarkers-Cyberpunk-mod) |
| `equipment-ex` | 1.2.10 | manual | [Nexus 6945](https://www.nexusmods.com/cyberpunk2077/mods/6945) |
| `browser-extension` | 0.9.7 | manual | [Nexus 10038](https://www.nexusmods.com/cyberpunk2077/mods/10038) |
| `no-intro-videos` | 0.8 | manual | [Nexus 533](https://www.nexusmods.com/cyberpunk2077/mods/533) |
| `virtual-atelier` | 1.6.4 | manual | [Nexus 2987](https://www.nexusmods.com/cyberpunk2077/mods/2987) |

The Cyber Engine Tweaks and Input Loader GitHub assets are byte-identical to
their Nexus files. The BetterLootMarkers GitHub release 1.4.1 is newer than
the Nexus 1.4.0 file that was inspected.
Equipment-EX is published as manual because the newest GitHub release, 1.2.7
from 2024, predates game 2.5; 1.2.10 is on Nexus only.

## Declared executables

`redscript` declares `engine/tools/scc.exe`, the script compiler shipped in its
original Windows ZIP. No other package installs an executable.

## Dependencies

Dependencies list what the upstream author documents as required, plus
anything the inspected archive needs to load. Optional integrations are left
out. Nexus requirement tables block automated fetches; the Nexus entries below
were read by the reviewer of this snapshot.

- `cet`: the v1.38.1 README says to install RED4ext first.
- `archivexl`, `tweakxl`, `codeware`: their READMEs require RED4ext 1.31.0+
  and list redscript 0.5.32+ for compatibility. Each ships `.reds` scripts.
  Their manifests declare both as `dependency_versions`.
- `mod-settings`: its README requires RED4ext and redscript. It also ships
  `ModSettings.archive.xl`, which ArchiveXL loads.
- `equipment-ex`: its README requires ArchiveXL, TweakXL, Codeware and
  redscript. Its scripts check for ArchiveXL 1.28.0 and TweakXL 1.12.0, which
  its manifest declares as `dependency_versions`.
- `virtual-atelier`: imports `Codeware.UI` unconditionally and ships an
  `.archive.xl`, so it needs Codeware and ArchiveXL. Its Nexus page also
  requires Mod Settings, which configures it through `@runtimeProperty`
  annotations. Browser Extension, Equipment-EX and Atelier Delivery support
  is `@if(ModuleExists(...))`, so it is optional.
- `browser-extension`: its Nexus page requires redscript, RED4ext and
  Codeware. Its Mod Settings support is conditional.
- `no-intro-videos`: one redscript source file.
- `input-loader`: its README says to install RED4ext first.
- `better-loot-markers`: its README requires Cyber Engine Tweaks 1.34.1+, which
  its manifest declares as a `dependency_version`.

Let There Be Flight (GitHub `jackhumbert/let_there_be_flight`, 0.3.20) is not
in the bucket. Its ZIP ships `red4ext/plugins/let_there_be_flight_uninstall.bat`,
and Modn blocks it as an unapproved executable payload in an unrecognized
layout (`EXECUTABLE_PAYLOAD`, `UNRECOGNIZED_LAYOUT`).

No upstream source documents a conflict between these packages, so no manifest
declares `conflicts`.

## Compatibility

The ArchiveXL, TweakXL and Codeware READMEs target Cyberpunk 2077 2.5. The CET
and Mod Settings Nexus files are labeled for game 2.5/2.50. On 2026-11-02 all
eleven packages installed, launched and uninstalled on a live Steam
installation that reports game version 3.0.88.46235 (project memory
finding `PM-20261102-ade7fa3a`).

The Input Loader and BetterLootMarkers packages were exercised only in the
disposable fixture below, not on a live installation.

In a disposable fixture, all thirteen archives install through the first-party
Cyberpunk policy in dependency order: 147 owned files. The eleven original
archives own 137 of them.
Uninstalling in reverse order restores the fixture. The milestone 3 gate
(`apps/modn-cli/tests/milestone_gate.rs`) repeats this through catalog stack
installs, which download the `https` archives, and adds rollback and recovery
checks. It compares the whole tree, directories included: uninstall and
rollback also remove the empty directories that Modn created.
