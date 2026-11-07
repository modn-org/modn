# Releasing

The `Release` workflow publishes only when a maintainer pushes a `v*` tag. The tag
must equal the `modn-cli` version, for example `v0.3.0`. The workflow builds
`modn.exe` on Windows and smoke-tests it on a second, clean runner. The smoke
test runs from an empty directory, with no repository and no `cargo` or `rustc`
on `PATH`. It checks the checksum, the absence of C runtime DLL imports,
`search`, `install --dry-run` for a catalog mod and for a local archive, and
`doctor` on a fake game. It also checks that the fake game's files and their
contents are unchanged. The smoke test installs the pinned `buckets/main`
submodule, packed with that repository's `pack.py`. Only
then does the workflow publish the GitHub release with `modn.exe`, its checksum
and `install.ps1`. Bucket releases come from `modn-org/main`. A version with a pre-release
suffix, such as `0.2.0-alpha.1`, becomes a GitHub pre-release. Pull requests
that change the workflow or the smoke script run the build and smoke jobs, but
they never publish.

Run the same smoke test locally on a built executable:

```powershell
$env:RUSTFLAGS = '-C target-feature=+crt-static'
cargo build --release --locked -p modn-cli
$hash = (Get-FileHash target/release/modn.exe -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText("$PWD/target/release/modn.exe.sha256", "$hash *modn.exe`n")
python buckets/main/.github/scripts/pack.py buckets/main target/smoke/main-bucket.zip
pwsh -File .github/scripts/release-smoke.ps1 -Binary target/release/modn.exe `
  -Checksum target/release/modn.exe.sha256 -MainBucket target/smoke/main-bucket.zip
```

## Milestone 1 exit gate

After a release, run the gate against its published assets:

```powershell
pwsh -File .github/scripts/milestone1-gate.ps1 -Tag v0.2.0
```

The script downloads `modn.exe`, its checksum and `install.ps1` from the
release. It removes `cargo` and `rustc` from `PATH` and keeps all state in a
temporary directory. It then:

1. bootstraps the `main` bucket with the install script and repeats
   `bucket update`
2. installs `archivexl` with its `red4ext` and `redscript` prerequisites on a
   fake game
3. previews and applies an update, using a local bucket with three releases of
   one package
4. checks that an unmanaged file blocks an update and that a locked file makes
   an update fail with the game unchanged
5. checks that a pinned package is not updated
6. opens state that the previous release created

The gate never touches a real game. A live-game update needs separate user
authorization.

## Milestone 2 exit gate

Run the gate against published release assets:

```powershell
pwsh -File .github/scripts/milestone2-gate.ps1 -Tag v0.3.0
```

Before release, build the static executable and checksum with the smoke-test
commands above. Then run:

```powershell
pwsh -File .github/scripts/milestone2-gate.ps1 `
  -Binary target/release/modn.exe -Checksum target/release/modn.exe.sha256 `
  -VerifierBinaries target/release
```

The gate verifies the checksum and calls the existing Milestone 1 gate.
The local mode bootstraps the bucket directly with the selected executable.
It cannot test an installer from an unpublished release.
The tag mode retains the released installer check.
Both modes download the previous release for the Milestone 1 state-compatibility check.

The additional checks use disposable game installations:

1. Sync a local Git catalog twice and resolve its package.
2. Install pinned redscript bytes through a `github-release` source, then uninstall them.
3. Refuse incomplete adoption approvals, then adopt matching existing bytes with explicit approval.
4. Refuse strict removal of modified owned bytes, then remove them with approved cleanup.
5. Install and uninstall canonical REDmod content without running a game or deployment tool.
6. Refuse a fixture main bucket that requires Modn 99.0.0, and preserve the previous bucket.
7. Run the existing MCP verifier and both desktop verification drivers.

The Release build job builds four verifier binaries from the same commit as `modn.exe`.
All five binaries use release optimization and a static C runtime.
The workflow stores the verifiers and their SHA-256 files in `modn-verifiers-<tag>` for 30 days.
Pull request runs also build and upload the verifiers, but never publish a release.
The GitHub release contains only `modn.exe`, its checksum, and `install.ps1`.

Tag mode requires an authenticated `gh` CLI with Actions read access.
It selects a successful Release workflow run for the tag and checks its commit against the tag commit.
It downloads that run's verifier artifact and verifies all four SHA-256 files before execution.
It does not use local build directories.
Run the gate before the artifact expires.
If the artifact is unavailable, tag mode fails.

Local mode requires an explicit `-VerifierBinaries <directory>` argument.
That directory must contain `modn-mcp.exe`, `modn-mcp-fault.exe`,
`modn-desktop.exe`, and `modn-desktop-test-backend.exe`.
Each binary needs a `<name>.sha256` file.
Build all prerequisites from the same revision before running the local gate:

```powershell
$env:RUSTFLAGS = '-C target-feature=+crt-static'
cargo build --release --locked -p modn-cli -p modn-mcp -p modn-desktop --features modn-desktop/verification
rustc -C opt-level=3 -C target-feature=+crt-static apps/modn-mcp/tests/fixtures/backend.rs -o target/release/modn-mcp-fault.exe
Get-ChildItem target/release/modn*.exe | ForEach-Object {
  $hash = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
  [IO.File]::WriteAllText("$($_.FullName).sha256", "$hash *$($_.Name)`n")
}
```

Local checksums detect changed bytes. They do not certify build provenance.
The refusal check expects the tag version in tag mode and the checkout version in local mode.

Install Python 3, Git, 7-Zip, Node.js, and `agent-desktop` before running the gate.
Desktop verification requires Windows 10 or newer and an unlocked interactive desktop.
The desktop build doctor requires Rust and build tools, so the gate does not run it.
The retained drivers run against prepared binaries without Rust on PATH.
Missing prerequisites fail the gate; they are never reported as passing checks.
Network access is required for GitHub metadata, bucket assets, and mod archives.
GitHub service failures and rate limits can fail a run.

The gate removes `cargo` and `rustc` from PATH before testing.
It overrides runtime data, state, cache, temporary, and desktop automation roots.
It clears inherited module and bucket overrides.
Both gates remove inherited `GIT_*` variables before any Git call.
They use a temporary Git configuration, disable system configuration, and select an empty hooks directory.
They restore the caller's Git environment on exit.
The Milestone 2 isolation regression checks an outside index, a hostile hook sentinel, and environment restoration.
All game writes use fixtures beneath its printed temporary directory.
It retains logs, fixture files, and `summary.json` there for inspection.
Remove that printed directory after inspecting the evidence.
It compares the original Modn roots before and after the run.
Each check prints `PASS` or `FAIL`. Any failure produces a nonzero exit.
These fixture checks do not establish real-game compatibility or REDmod activation.
The gate must pass against published assets after tagging; local proof alone does not close issue #87.

### Result for v0.3.0

On 2026-11-05, `milestone2-gate.ps1 -Tag v0.3.0` passed against the published v0.3.0 assets.
It reported 15 checks and 0 failures.
The Milestone 1 gate passed for v0.3.0, including state created by v0.1.0.
The MCP verifier passed 69 checks.
The desktop verifiers passed 51 and 8 assertions.
The refusal check printed `module cyberpunk-2077 needs Modn 99.0.0, you have 0.3.0`.
The original Modn roots were unchanged.
The released `modn.exe` SHA-256 is `2dade4e26e3db81ebf2dd62f0fc38a487184fc6968139dce43f1fd4bfa90eadb`.

## Real-game validation

The gates above use disposable fixtures and prove no real-game compatibility or REDmod activation.
The Milestone 3 exit gate needs live evidence from Windows 10 and Windows 11.
Run [Cyberpunk 2077 (#88)](#cyberpunk-2077-88) on Windows 11 and
[Windows 10 (#89)](#windows-10-89) on Windows 10.
[Balatro](#balatro) covers the native injector, the Git source and the content root.

### Before any live step

Run every command in PowerShell. Set these values once and record them:

```powershell
$tag = (gh release view --repo modn-org/modn --json tagName --jq .tagName)
$live = "$env:USERPROFILE\modn-live"
$game = "<game-root>"
New-Item -ItemType Directory -Force $live | Out-Null
```

**Release.** Use the latest published release for the whole run, `v0.3.0` at the time of writing.
Do not mix binaries from different tags. Download and verify it:

```powershell
gh release download $tag --repo modn-org/modn --pattern modn.exe --pattern modn.exe.sha256 --dir "$live\release"
(Get-FileHash "$live\release\modn.exe" -Algorithm SHA256).Hash
Get-Content "$live\release\modn.exe.sha256"
gh api "repos/modn-org/modn/commits/$tag" --jq .sha
```

Expect the two hashes to match, in any case, and a full commit SHA.
Record the tag, the commit and the SHA-256. Install Python 3, Git, `gh` and 7-Zip on `PATH`.

**OS version.** Run these commands and record the results:

```powershell
[System.Environment]::OSVersion.Version
winver
```

Record the caption, version, full build, architecture and whether the machine is physical or virtual.

**Game version and store.** Record the store and both version values:

```powershell
(Get-Item -LiteralPath "$game\bin\x64\Cyberpunk2077.exe").VersionInfo | Select-Object ProductVersion, FileVersion
```

**Protect user data.** Do this before step 1. Choose one option and record the choice:
- A full copy of the game installation on another disk, with its path and time.
- A SHA-256 manifest. The store's "verify integrity of game files" action is then the restore path.

```powershell
Get-ChildItem -LiteralPath $game -Recurse -File | Get-FileHash -Algorithm SHA256 |
  Select-Object Hash, Path | Set-Content -LiteralPath "$live\baseline.txt"
```

If a file cannot be read, stop. If the user declines both options, record the accepted risk and the restore path.

**Live-step rules.**
- Each live step starts with an **Authorize this step** checkpoint, for that step only.
  Preview each install and update with `--dry-run` before the write.
- Stop on a reported conflict, blocker or unmanaged destination file.
  Never delete or overwrite that file. Record the command, exit code and message.
- Stop on a pending transaction. Run `modn recover --yes` only after the user authorizes it.
- Never run a step that removes or replaces unmanaged content.

Every command and flag below exists in `apps/modn-cli/src/commands.rs`; check one with `modn <command> --help`.

### Cyberpunk 2077 (#88)

This run closes [issue #88](https://github.com/modn-org/modn/issues/88).
It covers install, update, rollback, uninstall, adoption, owned cleanup and one REDmod content package.
Use a clean game installation with no unmanaged framework or mod files.

#### Step 1: Register the installation and record the baseline

**Authorize this step.** Modn reads the game installation and writes only its own state.

```powershell
modn bucket update
modn game add cyberpunk-2077 --path $game
modn doctor --path $game
modn --game cyberpunk-2077 scan --json --path $game | Set-Content "$live\scan-before.json"
```

Expect `Installed main <digest>; changed: <bool>`, `Registered <path> (<source>)`,
`Detected unmanaged entries: <n>` and one `UNMANAGED` line per entry.
Record the entries and do not remove them.
Stop if the game already has CET, RED4ext, redscript or the mods below.

#### Step 2: Install a catalog stack and check the game

**Authorize this step.** Modn downloads and writes `better-loot-markers`, `cet` and `red4ext`.

```powershell
modn install better-loot-markers --dry-run --download --path $game --json | Set-Content "$live\install-plan.json"
modn install better-loot-markers --yes --path $game
modn list --path $game --json | Set-Content "$live\list-after-install.json"
```

Expect `"status": "reviewable"`, `"metadata_only": true` and a `main/cyberpunk-2077/cet` dependency.
Expect one `Installed <reference> (<n> owned files)` line for each of the three
packages. The `list` JSON has one catalog reference each for `red4ext`, `cet` and
`better-loot-markers`, as `{"bucket": "main", "game_id": "cyberpunk-2077", "mod_id": "<id>"}`.

Game-side check: launch the game, open the CET overlay and confirm the loot markers render.
Record the overlay key, the marker and the game version, then quit.

#### Step 3: Update and roll back a catalog mod

**Authorize this step.** Modn replaces ArchiveXL 1.28.6 with 1.28.5 and back through a local bucket.
Each update writes game files. The official bucket publishes the current release only.
Add 1.28.5 to a local bucket:

```powershell
$bucket = "$live\bucket"
$main = "<data dir>\buckets\main"  # the official bucket copy; see buckets.md
New-Item -ItemType Directory -Force "$bucket\games\cyberpunk-2077" | Out-Null
'{"schema_version":1,"name":"live","title":"Live validation"}' | Set-Content "$bucket\bucket.json"
'archivexl','red4ext','redscript' | ForEach-Object {
  Copy-Item "$main\games\cyberpunk-2077\$_.json" "$bucket\games\cyberpunk-2077\"
}
$url = 'https://github.com/psiberx/cp2077-archive-xl/releases/download/v1.28.5/ArchiveXL-1.28.5.zip'
Invoke-WebRequest $url -OutFile "$live\ArchiveXL-1.28.5.zip"
$old = Get-Content "$bucket\games\cyberpunk-2077\archivexl.json" -Raw | ConvertFrom-Json
$old.version = '1.28.5'; $old.source.url = $url
$old.sha256 = (Get-FileHash "$live\ArchiveXL-1.28.5.zip" -Algorithm SHA256).Hash.ToLowerInvariant()
$old | ConvertTo-Json -Depth 5 | Set-Content "$bucket\games\cyberpunk-2077\archivexl@1.28.5.json"
modn bucket add live $bucket
```

Expect the registered bucket as JSON and exit 0.

Install the current release, roll back, update forward and roll back again:

```powershell
modn install live/cyberpunk-2077/archivexl --dry-run --download --path $game --allow-executable engine/tools/scc.exe --json
modn install live/cyberpunk-2077/archivexl --yes --path $game --allow-executable engine/tools/scc.exe
modn update live/cyberpunk-2077/archivexl --version 1.28.5 --dry-run --download --path $game --json
modn update live/cyberpunk-2077/archivexl --version 1.28.5 --yes --path $game
modn update live/cyberpunk-2077/archivexl --version 1.28.6 --dry-run --download --path $game --json
modn update live/cyberpunk-2077/archivexl --version 1.28.6 --yes --path $game
modn update live/cyberpunk-2077/archivexl --version 1.28.5 --dry-run --download --path $game --json
modn update live/cyberpunk-2077/archivexl --version 1.28.5 --yes --path $game
modn list --path $game --json | Set-Content "$live\list-after-rollback.json"
```

Expect `Installed` for `redscript` and `archivexl`, in that order.
Modn prints no `Installed` line for `red4ext`, because Step 2 installed the same
archive digest.
Each preview names the installed and target versions; each update prints
`Updated live/cyberpunk-2077/archivexl to <version> (<n> owned files)`.
The final `list` shows archivexl with the 1.28.5 digest.

Game-side check: launch the game, confirm the CET overlay opens and record the newest
ArchiveXL log line under `red4ext/plugins/ArchiveXL/`. Quit the game.

#### Step 4: Adopt manual files and clean them up

**Authorize this step.** Modn previews an archive, then claims files that you copy into the game.
Adoption changes no game bytes. Download No Intro Videos 0.8 from Nexus mod 533.

```powershell
$adopt = "<path-to-No Intro Videos - redscript-533-0-8-1760416944.zip>"
modn adopt $adopt --dry-run --path $game --json | Set-Content "$live\adopt-preview.json"
```

Expect `"status": "verification_required"`, `"target_contents_verified": false` and one operation
per destination. Stop if a destination already exists.

Copy only the planned files, then commit the adoption:

```powershell
7z x $adopt -o"$live\adopt-staging" -y
$plan = Get-Content "$live\adopt-preview.json" -Raw | ConvertFrom-Json
$taken = $plan.plan.operations | Where-Object { Test-Path -LiteralPath (Join-Path $game $_.destination) }
if ($taken) { throw "Stop: destinations already exist: $($taken.destination -join ', ')" }
foreach ($op in $plan.plan.operations) {
  $to = Join-Path $game $op.destination
  New-Item -ItemType Directory -Force (Split-Path $to) | Out-Null
  Copy-Item -LiteralPath (Join-Path "$live\adopt-staging" $op.source) -Destination $to
}
modn adopt $adopt --acknowledge --yes --path $game
modn list --path $game --json | Set-Content "$live\list-after-adopt.json"
```

Expect `Adopted <package-id> (<n> individual owned files; game bytes unchanged)` and `"catalog": null`;
`scan` reports the files as managed.

Game-side check: confirm that the intro videos do not play. Record it and quit.

Append one byte to one adopted file, read the package ID from `list-after-adopt.json`, then run:

```powershell
modn uninstall <package-id> --yes --path $game
modn uninstall <package-id> --force --dry-run --path $game
modn uninstall <package-id> --force --yes --path $game
```

Expect the strict uninstall to exit nonzero and name the modified file, and the force removal to print
`Uninstalled <package-id>`. Unrecorded files stay.

#### Step 5: Uninstall the remaining managed packages

**Authorize this step.** Modn removes only owned files and exclusive directories from this run.
Copy any CET setting that you want to keep before this step.

```powershell
modn uninstall live/cyberpunk-2077/archivexl --include-deps --dry-run --path $game
modn uninstall live/cyberpunk-2077/archivexl --include-deps --yes --path $game
modn uninstall better-loot-markers --include-deps --dry-run --path $game
modn uninstall better-loot-markers --include-deps --yes --path $game
modn list --path $game
```

Expect `Would uninstall, in this order:` from each dry run; record the order.
Expect `Uninstalled <id>` for each package and a final `No manager-owned packages installed.`

Game-side check: launch the game and reach the main menu without the mods. Quit.

#### Step 6: Install and uninstall one REDmod content package

**Authorize this step.** Modn writes and then removes one standalone REDmod package.
Pick a REDmod mod whose archive holds exactly one `mods/<name>/info.json`, with optional files
under `archives`, `scripts` and `tweaks` in that folder.
The official bucket has no REDmod package.
Reject an archive with several names, noncanonical casing or a mixed layout.

```powershell
$redmod = "<path-to-REDmod.zip>"
modn inspect $redmod
modn install $redmod --dry-run --path $game --json | Set-Content "$live\redmod-plan.json"
modn install $redmod --yes --path $game
modn list --path $game --json | Set-Content "$live\list-after-redmod.json"
modn --game cyberpunk-2077 scan --json --path $game | Set-Content "$live\scan-after-redmod.json"
modn doctor --path $game
```

Expect `inspect` to list one `mods/<name>/info.json`, a `"reviewable"` preview and
`Installed <package-id> (<n> owned files)` with `"catalog": null`.
`scan` reports the `mods/<name>` folder as `redmod-directory` and unmanaged.
`doctor` prints `UNMANAGED_CONTENT` for that folder; this is expected.

Game-side check: reach the main menu and record "REDmod activation not verified".
Modn does not deploy REDmod. Do not run the deployment tool during this run.
See [REDmod content](lifecycle.md#redmod-content) for the policy limits.

```powershell
modn uninstall <package-id> --dry-run --path $game
modn uninstall <package-id> --yes --path $game
modn list --path $game
```

Expect `Uninstalled <package-id>` and `No manager-owned packages installed.`
Runtime files under `mods/<name>` stay unmanaged. Record each remaining path.

#### Step 7: Final cleanup and comparison

**Authorize this step.** Modn forgets the local bucket and the game registration. Neither deletes a game file.

```powershell
modn bucket remove live
modn --game cyberpunk-2077 scan --json --path $game | Set-Content "$live\scan-after.json"
modn doctor --path $game
modn game remove cyberpunk-2077 --path $game
```

Expect `Unregistered cyberpunk-2077 (<path>)`, no managed files and only runtime files.
Compare the game tree with the baseline:

```powershell
Get-ChildItem -LiteralPath $game -Recurse -File | Get-FileHash -Algorithm SHA256 |
  Select-Object Hash, Path | Set-Content "$live\after.txt"
Compare-Object (Get-Content "$live\baseline.txt") (Get-Content "$live\after.txt")
```

Every difference must be a runtime file.
A missing or changed owned file is a difference from the fixture results. Record it and file a bug.

#### What to record for #88

Write these facts into one record:

```text
Cyberpunk 2077 live validation, <date>
- Release <tag>, commit <sha>, modn.exe SHA-256 <hash>, state <path>
- OS <caption> <version>, build <build>, <physical|VM>
- Game <version>, <Steam|GOG|Epic>, installation <path>
- Backup <full copy at ... | manifest at ...>; risk acceptance <yes, restore <...> | not needed>
- Baseline <n> unmanaged entries, <n> hashed files

| Step | Command | Exit | Key output line | Game-side check |
| --- | --- | --- | --- | --- |
| 1 | modn game add ... | 0 | Registered ... | none |
| ... | ... | ... | ... | ... |

Differences from fixture results
- <none, or command, observed value and fixture value>
```

### Balatro

This run records the first live check of the Balatro packages.
It covers a native injector, a Git source and an AppData content root.
Use the release from [Before any live step](#before-any-live-step) when it contains Balatro support.
Otherwise use the build from the Balatro support change.
Record its commit and SHA-256.
The `main` bucket must also contain the two Balatro packages.
At the time of writing, that bucket release is a draft pull request in `modn-org/main`.
Install that draft copy before the run.
Every command and flag below exists in the built CLI; check one with `modn <command> --help`.

Set these values once. Reuse `$live` and `$game` from [Before any live step](#before-any-live-step).

```powershell
$appdata = "$env:APPDATA\Balatro"
$mods = "$appdata\Mods"
```

The game root is the Steam folder that contains `Balatro.exe` (app `2379780`).
The content root is `%APPDATA%\Balatro\Mods`.
Modn never reads or writes the save folders `1`, `2` and `3`, or the game settings.
The backup in Step 2 covers the whole `%APPDATA%\Balatro` folder.

#### Step 1: Confirm Balatro support

**Authorize this step.** This step reads local files and prints catalog entries. It writes no game file.

```powershell
modn bucket list
modn --game balatro search
git --version
(Get-Command modn).Source
(Get-FileHash (Get-Command modn).Source -Algorithm SHA256).Hash
```

Expect `main: <title>` from `bucket list`.
Expect `main/balatro/lovely, version 0.10.0` and a line that names `main/balatro/steamodded`.
Expect a `git version <n>` line and a 64-character hash for the Modn binary.
Record the bucket source, the Modn path, the hash and the Git version.
Stop if either package line is absent. Stop if Git is not on `PATH`.
Do not run `modn bucket update` on a draft bucket. It can replace the draft copy.

#### Step 2: Back up the user data and existing Lovely files

**Authorize this step.** This step reads the Balatro folders and writes only under `$live`.
Close Balatro before the copy.

```powershell
if (Test-Path -LiteralPath $appdata) {
  if (Test-Path -LiteralPath "$live\balatro-appdata-backup") { throw "Stop: backup destination exists" }
  Copy-Item -LiteralPath $appdata -Destination "$live\balatro-appdata-backup" -Recurse
  "AppData backup: $live\balatro-appdata-backup"
} else {
  "AppData absent: $appdata"
}
$before = @(Get-ChildItem -LiteralPath $mods -Recurse -File -ErrorAction SilentlyContinue |
  Get-FileHash -Algorithm SHA256 | ForEach-Object { "$($_.Hash) $($_.Path)" })
$before | Set-Content "$live\mods-before.txt"
"Mods present: $(Test-Path -LiteralPath $mods)"
"Mods baseline: $($before.Count) files"
if (Test-Path -LiteralPath "$live\balatro-game-dlls") { throw "Stop: DLL backup destination exists" }
New-Item -ItemType Directory -Force "$live\balatro-game-dlls" | Out-Null
foreach ($dll in 'winmm.dll','version.dll') {
  $path = Join-Path $game $dll
  if (Test-Path -LiteralPath $path) {
    Copy-Item -LiteralPath $path -Destination "$live\balatro-game-dlls\$dll"
    "$dll present; SHA-256 $((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash)"
  } else {
    "$dll absent"
  }
}
```

Expect one `AppData ...` line, one `Mods present: <bool>` line, one `Mods baseline: <n> files` line and one line for each DLL.
Record the backup path, the time and both DLL results.
Stop if any copy fails.
The game can change saves and settings when it runs. Compare only `Mods` and the two DLLs.
A present `winmm.dll` or `version.dll` is an unmanaged Lovely installation.
Complete Step 3 to record its state, then stop. Do not overwrite it.

#### Step 3: Register, scan and check the Lovely state

**Authorize this step.** Modn reads both roots and writes only its own state.

```powershell
modn game add balatro --path $game
modn --game balatro scan --path $game | Tee-Object "$live\scan-before.txt"
modn --game balatro doctor --path $game | Tee-Object "$live\doctor-before.txt"
```

Expect `Registered <path> (manual)`.
The source is `manual` because the command passes `--path`.
Expect these scan lines:

```text
balatro: <path>
Content root: <APPDATA>\Balatro\Mods
REDmod executable present: false
```

Expect these doctor lines:

```text
balatro: <path>
REDmod executable present: false
Detected unmanaged entries: <n>
Owned files checked: 0
Folders Modn created: 0
```

Expect a `LOVELY_MISSING` diagnostic on a clean installation.
A missing `Mods` folder also reports `CONTENT_ROOT_MISSING`. This is expected before the first install.
Record the unmanaged entries and the doctor diagnostics. Do not remove unmanaged content.

Stop the run in these cases:
- The doctor reports `LOVELY_PRESENT` or `LOVELY_LEGACY`.
- `winmm.dll` or `version.dll` exists in the game folder.
- The doctor reports `CONTENT_ROOT_LINKED`, `CONTENT_ROOT_NOT_DIRECTORY`, `CONTENT_ROOT_MOVED` or `CONTENT_ROOT_UNAVAILABLE`.

Record the state and the Lovely file. Do not install Lovely over an unmanaged copy.
An incomplete scan cannot prove the game is clean. Record it and use the destination check.
If the run stops here, unregister the game with `modn game remove balatro --path $game`. That command deletes no file.

#### Step 4: Refuse and preview Lovely

**Authorize this step.** The first command must fail. The second downloads Lovely into the cache. Neither writes a game file.

```powershell
modn --game balatro install main/balatro/lovely --dry-run --path $game --json 2> "$live\lovely-refusal.json"
if ($LASTEXITCODE -ne 1) { throw "Stop: expected a native injector refusal" }
Get-Content "$live\lovely-refusal.json"
```

Expect exit 1 and a `NATIVE_INJECTOR_NOT_ALLOWED` error.
The error names `main/balatro/lovely` and the `--allow-native-injector` flag.
No download and no game write occur.
Stop if the command succeeds. Stop if the error code is different.

Then preview the allowed install:

```powershell
modn --game balatro install main/balatro/lovely --dry-run --download --allow-native-injector --path $game --json | Set-Content "$live\lovely-plan.json"
```

Expect `"version": "0.10.0"` and `"status": "reviewable"`.
Expect one operation with `"destination": "winmm.dll"` and `"root": "game"`.
Expect no blockers and no conflicts.
Check every planned destination before the install:

```powershell
$plan = Get-Content "$live\lovely-plan.json" -Raw | ConvertFrom-Json
if (-not $plan.plan -or $plan.plan.status -ne 'reviewable') { throw "Stop: no reviewable plan" }
$roots = @{ game = $game; content = $mods }
$taken = $plan.plan.operations | Where-Object { Test-Path -LiteralPath (Join-Path $roots[$_.root] $_.destination) }
if ($taken) { throw "Stop: destinations already exist: $($taken.destination -join ', ')" }
```

Expect no output from the check. Stop on any output.
Never overwrite an existing file.

#### Step 5: Install Lovely

**Authorize this step.** Modn writes `winmm.dll` beside `Balatro.exe`.

```powershell
modn --game balatro install main/balatro/lovely --yes --allow-native-injector --path $game
modn --game balatro list --path $game
modn --game balatro doctor --path $game
```

Expect `Installed main/balatro/lovely (<n> owned files)`.
Expect `list` to show `<package-id>: main/balatro/lovely, <n> owned files`.
Expect the doctor to report `LOVELY_PRESENT`.
Record the package ID and `n`.
Close Balatro before this step. Do not run it during a write.

#### Step 6: Preview and install Steamodded

**Authorize this step.** The preview fetches the pinned Git commit. The install writes only `smods` files under `Mods`.

```powershell
modn --game balatro install main/balatro/steamodded --dry-run --download --path $game --json | Set-Content "$live\steamodded-plan.json"
```

Expect `"status": "reviewable"` and `"dependencies_ready": true`.
Expect a dependency record for `main/balatro/lovely` at version `0.10.0` in the `ready` state.
Expect the Git source to pin commit `af0fdb1eb2c9e191262c128bc88bbc4da991e12d` (tag `26.1002.0`).
Expect each operation to have `"root": "content"` and a destination under `smods/`.
The dry run needs `--download` to fetch the Git tree. It writes no game file.
Stop if `plan.blockers` is not empty or a dependency is not ready.

Check every planned destination:

```powershell
$plan = Get-Content "$live\steamodded-plan.json" -Raw | ConvertFrom-Json
if (-not $plan.plan -or $plan.plan.status -ne 'reviewable') { throw "Stop: no reviewable plan" }
$roots = @{ game = $game; content = $mods }
$taken = $plan.plan.operations | Where-Object { Test-Path -LiteralPath (Join-Path $roots[$_.root] $_.destination) }
if ($taken) { throw "Stop: destinations already exist: $($taken.destination -join ', ')" }
```

Expect no output. Stop on any output.
A plan that claims an existing exclusive folder is not reviewable. Stop.
Unmanaged mods outside these paths stay untouched.

Then install:

```powershell
modn --game balatro install main/balatro/steamodded --yes --download --path $game
modn --game balatro list --path $game
```

Expect `Installed main/balatro/steamodded (<n> owned files)`.
Expect no `Installed` line for Lovely, because that dependency is ready.
Expect `list` to show both packages. Record the package ID and `n`.
Pass `--download` on the install. The preview and the install then use the same explicit fetch.

#### Step 7: Confirm that Steamodded loads

**Authorize this step.** This step starts Balatro. The game writes saves and settings, which the backup covers.

Launch Balatro through Steam.
Expect a `Mods` entry in the main menu. Open it and record the Steamodded version.
Quit the game before any later step.
If the entry is absent, stop. Record the result and keep all files for inspection.
Do not reinstall over the installed tree.

#### Step 8: Uninstall Steamodded and Lovely

**Authorize this step.** Modn removes only files that it installed for this run.

```powershell
modn --game balatro uninstall main/balatro/steamodded --dry-run --path $game
modn --game balatro uninstall main/balatro/steamodded --yes --path $game
modn --game balatro uninstall main/balatro/lovely --dry-run --path $game
modn --game balatro uninstall main/balatro/lovely --yes --path $game
modn --game balatro list --path $game
```

Expect `Would uninstall, in this order:` from each dry run, with one package line.
Expect `Uninstalled <package-id>` for each removal.
Expect `No manager-owned packages installed.` from the final `list`.
Remove Steamodded first. Lovely is its dependency.

Then compare both roots with the baseline:

```powershell
$after = @(Get-ChildItem -LiteralPath $mods -Recurse -File -ErrorAction SilentlyContinue |
  Get-FileHash -Algorithm SHA256 | ForEach-Object { "$($_.Hash) $($_.Path)" })
$before = if (Test-Path -LiteralPath "$live\mods-before.txt") { @(Get-Content "$live\mods-before.txt") } else { @() }
if ($before.Count -eq 0 -and $after.Count -eq 0) {
  "Mods restored: empty"
} elseif ($before.Count -eq 0 -or $after.Count -eq 0) {
  "DIFF before=$($before.Count) files, after=$($after.Count) files"
} else {
  Compare-Object $before $after
}
"Mods present: $(Test-Path -LiteralPath $mods)"
foreach ($dll in 'winmm.dll','version.dll') {
  if (Test-Path -LiteralPath (Join-Path $game $dll)) { "REMAINS $dll" } else { "absent $dll" }
}
```

Expect `Mods restored: empty` or no `Compare-Object` output.
Expect the same `Mods present` value as Step 2.
Expect `absent` for both DLLs.
A difference must be a runtime file that the game created. Record it.
A missing or changed unmanaged file is a failure. Keep the backup and file a bug.
Do not delete an unmanaged file without separate authorization.

#### Step 9: Clean up

**Authorize this step.** Modn unregisters the installation. It deletes no game or AppData file.

```powershell
modn game remove balatro --path $game
```

Expect `Unregistered balatro (<path>)`.
Keep `$live` and `$live\balatro-appdata-backup` until you write the record and every comparison passes.
Then remove `$live`. If a check failed, keep `$live` for the bug report.
Do not delete `%APPDATA%\Balatro`, `Mods` or any unmanaged file during cleanup.

#### What to record for Balatro

```text
Balatro live validation, <date>
- Modn <tag or build>, commit <sha>, modn.exe SHA-256 <hash>, state <path>
- main bucket <digest or source>; search <main/balatro/lovely, version 0.10.0 | missing>
- OS <caption> <version>, build <build>, <physical|VM>
- Game <version>, Steam app 2379780, installation <path>
- Steamodded tag 26.1002.0, commit af0fdb1eb2c9e191262c128bc88bbc4da991e12d
- Backup %APPDATA%\Balatro at <path>, <time>; Mods <present|absent>; DLLs <none|winmm.dll|version.dll, backed up at ...>
- Baseline <n> files under Mods; unmanaged entries <n>

| Step | Command | Exit | Key output line | Game-side check |
| --- | --- | --- | --- | --- |
| 1 | modn --game balatro search | 0 | main/balatro/lovely, version 0.10.0 | none |
| 2 | Copy-Item ... | 0 | AppData backup: ... | none |
| 3 | modn --game balatro doctor | 0 | LOVELY_MISSING | game not launched |
| 4 | modn ... --dry-run (no flag) | 1 | NATIVE_INJECTOR_NOT_ALLOWED | none |
| 5 | modn ... lovely --yes | 0 | Installed main/balatro/lovely (<n> owned files) | none |
| 6 | modn ... steamodded --yes | 0 | Installed main/balatro/steamodded (<n> owned files) | none |
| 7 | Steam launch | n/a | Mods menu present | Mods menu opened |
| 8 | modn ... uninstall | 0 | Uninstalled <id>, No manager-owned packages installed. | none |
| 9 | modn game remove balatro | 0 | Unregistered balatro (<path>) | none |

Differences from fixture results
- <none, or command, observed value and fixture value>
```

Record the Lovely refusal text, both package IDs, the owned-file counts and the `Mods` menu label.

### Windows 10 (#89)

This run closes [issue #89](https://github.com/modn-org/modn/issues/89) with the released `modn.exe`
from [Before any live step](#before-any-live-step) and verifier binaries from the same commit.
The `modn-verifiers-<tag>` workflow artifact holds the desktop binaries.
Download it before it expires, 30 days after the release.

#### Step 1: Confirm the OS build

**Authorize this step.** Allow read-only OS queries.

Run the OS version commands in [Before any live step](#before-any-live-step).
Expect `10.0.19045` for Windows 10 22H2 and a `Microsoft Windows 10` caption.
Stop if the build is 22000 or higher: that is Windows 11, not #89.

#### Step 2: Download and verify the verifier binaries

**Authorize this step.** Allow downloads into `$live`.

```powershell
$commit = (gh api "repos/modn-org/modn/commits/$tag" --jq .sha)
$run = gh run list --repo modn-org/modn --workflow release.yml --branch $tag --event push --status success --json databaseId,headSha --limit 20 | ConvertFrom-Json | Where-Object headSha -eq $commit | Select-Object -First 1
gh run download $run.databaseId --repo modn-org/modn --name "modn-verifiers-$tag" --dir "$live\verifiers"
Get-ChildItem "$live\verifiers\*.exe" | ForEach-Object {
  "{0}: {1}" -f $_.Name, ((Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() -eq ((Get-Content "$($_.FullName).sha256") -split '\s+')[0].ToLowerInvariant())
}
```

Expect `True` for all four verifier binaries and record each SHA-256 with the tag, commit and run ID.
Stop if an artifact is missing; do not use a local build.

#### Step 3: Run the Milestone 1 gate

**Authorize this step.** The gate uses release assets and a fake game in a temporary directory.

```powershell
git clone https://github.com/modn-org/modn.git "$live\modn"
git -C "$live\modn" checkout $tag
Set-Location "$live\modn"
pwsh -File .github/scripts/milestone1-gate.ps1 -Tag $tag
```

Expect exit 0, `ok:` for every check and a final `milestone 1 exit gate passed for <tag>`.
The gate uses the released `modn.exe`. See [Milestone 1 exit gate](#milestone-1-exit-gate).
Count the `ok:` lines and record the count.

#### Step 4: Run the desktop verifiers

**Authorize this step.** The app opens one window and writes only to unique `modn-desktop-fixture-*` directories.

Prerequisites: Python 3, 7-Zip, `agent-desktop` and an unlocked interactive desktop.
Skip the build doctor; the released drivers need no build toolchain.
If the automation state fails its DACL check, set a unique `AGENT_DESKTOP_HOME` for the whole run.

```powershell
agent-desktop skills get desktop > "$live\desktop-skill.json"
agent-desktop skills get platform > "$live\platform-skill.json"
agent-desktop status
python .agents/skills/verify-modn-desktop/scripts/verify.py --app "$live\verifiers\modn-desktop.exe" --backend "$live\release\modn.exe"
python .agents/skills/verify-modn-desktop/scripts/lifecycle.py --app "$live\verifiers\modn-desktop.exe" --backend "$live\verifiers\modn-desktop-test-backend.exe"
```

Expect exit 0 and a JSON summary with `"status": "passed"`, a positive `assertions` count,
`"command_nonzero": 0`, `"app_exit": 0` and `"fixture_removed": true` for each driver.
The v0.3.0 fixture run reported 51 and 8 assertions. Record both counts and keep the evidence path.

#### What to record for #89

Write these facts into one record:

```text
Windows 10 validation, <date>
- Release <tag>, commit <sha>, modn.exe SHA-256 <hash>, workflow run <id>
- Verifier SHA-256: modn-desktop.exe <hash>, test backend <hash>
- OS <caption> <version>, build <build>, <physical|VM>; gate game <fake fixture>

| Command | Exit | Key output line |
| --- | --- | --- |
| milestone1-gate.ps1 | 0 | milestone 1 exit gate passed for <tag>, <n> ok lines |
| verify.py | 0 | status: passed, <n> assertions, 0 failures |
| lifecycle.py | 0 | status: passed, <n> assertions, 0 failures |

Differences from fixture results
- <none, or command, observed value and fixture value>
```

### Record the result and file bugs

Add each record with the `project-wiki` skill, then commit it:

```powershell
uv run --no-project python "$env:USERPROFILE\.agents\skills\project-wiki\scripts\wiki.py" add `
  --kind finding --title "<run> passed on <date>" `
  --source "https://github.com/modn-org/modn/releases/tag/$tag" `
  --source "https://github.com/modn-org/modn/issues/<88|89>" `
  --body-file "$live\record.md"
uv run --no-project python "$env:USERPROFILE\.agents\skills\project-wiki\scripts\wiki.py" lint
```

Keep raw evidence outside the repository and cite its local path in the body.
File one bug for each difference from the fixture results:

```powershell
gh issue create --repo modn-org/modn --label bug --title "Live <game>: <difference>" --body-file "$live\bug.md"
```

Use this body:

```text
## Expected
<fixture result, with the gate or test that shows it>
## Actual
<live result, with the command and exit code>
## Reproduction
<commands>
## Environment
<release, OS build, game version and store, evidence path>
```
