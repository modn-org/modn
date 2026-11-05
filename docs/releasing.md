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

The Milestone 3 exit gate needs live evidence from Windows 10 and Windows 11.
The gates above use disposable fixtures only.
They do not establish real-game compatibility or REDmod activation.
Run the [Cyberpunk 2077 run](#cyberpunk-2077-88) on Windows 11 and the
[Windows 10 run](#windows-10-89) on Windows 10 hardware.
A Balatro subsection follows when its game module ships.

### Before any live step

Run every command in PowerShell. Set these values once and record them:

```powershell
$tag = (gh release view --repo modn-org/modn --json tagName --jq .tagName)
$live = "$env:USERPROFILE\modn-live"
$game = "<game-root>"
New-Item -ItemType Directory -Force $live | Out-Null
```

**Release.** Use the latest published release for the whole run.
Do not mix binaries from different tags.
At the time of writing, the latest release is `v0.3.0`.
Download and verify it:

```powershell
gh auth status
gh release download $tag --repo modn-org/modn --pattern modn.exe --pattern modn.exe.sha256 --dir "$live\release"
$hash = (Get-FileHash "$live\release\modn.exe" -Algorithm SHA256).Hash.ToLowerInvariant()
$expected = ((Get-Content "$live\release\modn.exe.sha256") -split '\s+')[0].ToLowerInvariant()
$hash -eq $expected
gh api "repos/modn-org/modn/commits/$tag" --jq .sha
```

Expect `True` and a full commit SHA.
Record the tag, the commit and the `modn.exe` SHA-256.

Install Python 3, Git, GitHub CLI (`gh`) and 7-Zip on `PATH` for both runs.

**OS version.** Record the Windows version and build before any modn command:

```powershell
[System.Environment]::OSVersion.Version
Get-CimInstance Win32_OperatingSystem | Select-Object Caption, Version, BuildNumber, OSArchitecture
winver
```

The `winver` dialog shows the edition and the full build.
Record the caption, version, build and architecture.
Record whether the machine is physical or a virtual machine.

**Game version and store.** Record the store (Steam, GOG or Epic) and the game
version:

```powershell
(Get-Item -LiteralPath "$game\bin\x64\Cyberpunk2077.exe").VersionInfo |
  Select-Object ProductVersion, FileVersion
```

The game's main menu or launcher also shows a version.
Record both values and the difference, if any.

**Back up the game.** Do this before step 1 of the Cyberpunk run.
Choose one option and record the choice:
- Full copy: copy the game installation to another disk with your backup tool.
  Record the backup path and time.
- Manifest only: record a SHA-256 manifest.
  The Steam "Verify integrity of game files" action is then the restore path.
  This is what the 2026-11-02 live run used.

```powershell
New-Item -ItemType Directory -Force $live | Out-Null
Get-ChildItem -LiteralPath $game -Recurse -File | ForEach-Object {
  $rel = $_.FullName.Substring($game.Length).TrimStart('\')
  "$((Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant())  $rel"
} | Set-Content -LiteralPath "$live\baseline.sha256"
(Get-Content "$live\baseline.sha256").Count
```

If a file cannot be read, stop and record the path.
Do not continue with a partial manifest.
If the user declines both options, record that the user accepts the risk and the
restore path in the run notes and the memory record.

**Live-step rules.**
- Every live step starts with an **Authorize this step** checkpoint.
  Ask the user for that step only. Wait for a clear yes.
- Preview each install and update with `--dry-run` before the write.
  Adoption has its own preview.
- Stop if a command reports a conflict, a blocker, or an unmanaged file at a
  destination. Do not delete or overwrite that file.
  Record the command, exit code and message.
- When a command leaves a pending transaction, stop.
  Run `modn recover --yes` only after the user authorizes recovery.
- Never run a step that removes or replaces unmanaged content.

Every `modn` command and flag below is defined in
`apps/modn-cli/src/commands.rs`. Confirm one with `modn <command> --help`.

### Cyberpunk 2077 (#88)

This run closes [issue #88](https://github.com/modn-org/modn/issues/88).
It covers install, update, rollback, uninstall, adoption and owned cleanup.
It also installs one REDmod content package.
Use a clean game installation with no unmanaged framework or mod files.

#### Step 1: Register the installation and record the baseline

**Authorize this step.** Modn reads the game folder and writes only its own
state. This step writes no game file.

```powershell
modn bucket update
modn game add cyberpunk-2077 --path $game
modn doctor --path $game
modn --game cyberpunk-2077 scan --json --path $game |
  Set-Content -LiteralPath "$live\scan-before.json"
```

Expected output:
- `Installed main <digest>; changed: <true|false>`
- `Registered <game> (<source>)`
- `cyberpunk-2077: <game>` and `Owned files checked: 0`
- `Detected unmanaged entries: <n>` with one `UNMANAGED` line per entry

Record the unmanaged entries. Do not remove them.
Stop if the game has CET, RED4ext, redscript or the mods below already present.

#### Step 2: Install a catalog stack and check the game

**Authorize this step.** Modn downloads two archives and writes them into the
real game. It installs `better-loot-markers` and its `cet` dependency.

```powershell
modn install better-loot-markers --dry-run --download --path $game --json |
  Set-Content -LiteralPath "$live\install-plan.json"
modn install better-loot-markers --yes --path $game
modn list --path $game --json |
  Set-Content -LiteralPath "$live\list-after-install.json"
```

Expected output:
- The preview has `"status": "reviewable"`, `"metadata_only": true`, and a
  `dependencies` entry for `main/cyberpunk-2077/cet`.
- The install prints one `Installed <reference> (<n> owned files)` line for
  `main/cyberpunk-2077/cet` and one for
  `main/cyberpunk-2077/better-loot-markers`.
- `list` shows both packages with their catalog references.

Game-side check: launch Cyberpunk 2077.
Open the CET overlay and confirm that the loot markers render in the game.
Record the overlay key, the marker you saw and the game version.
Quit the game before the next step.

#### Step 3: Update and roll back a catalog mod

**Authorize this step.** Modn registers one local bucket and replaces
ArchiveXL 1.28.6 with 1.28.5 and back. Each update writes game files.
The local bucket publishes two real ArchiveXL releases.

The official `main` bucket publishes only the current release.
Build the `live` bucket from the `main` manifests and add the 1.28.5 release:

```powershell
$bucket = "$live\bucket"
New-Item -ItemType Directory -Force "$bucket\games\cyberpunk-2077" | Out-Null
'{"schema_version":1,"name":"live","title":"Live validation"}' |
  Set-Content -LiteralPath "$bucket\bucket.json"
Get-ChildItem "buckets\main\games\cyberpunk-2077\*.json" |
  Where-Object Name -ne 'module.json' |
  Copy-Item -Destination "$bucket\games\cyberpunk-2077\"

$zip = "$live\ArchiveXL-1.28.5.zip"
Invoke-WebRequest "https://github.com/psiberx/cp2077-archive-xl/releases/download/v1.28.5/ArchiveXL-1.28.5.zip" -OutFile $zip
$old = Get-Content "$bucket\games\cyberpunk-2077\archivexl.json" -Raw | ConvertFrom-Json
$old.version = '1.28.5'
$old.sha256 = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
$old.source.url = 'https://github.com/psiberx/cp2077-archive-xl/releases/download/v1.28.5/ArchiveXL-1.28.5.zip'
$old | ConvertTo-Json -Depth 5 |
  Set-Content -LiteralPath "$bucket\games\cyberpunk-2077\archivexl@1.28.5.json"
modn bucket add live $bucket
```

Expected: `bucket add` prints the registered bucket as JSON and exits 0.

Install the current release, update it, then roll it back:

```powershell
modn install live/cyberpunk-2077/archivexl --dry-run --download --path $game --allow-executable engine/tools/scc.exe --json
modn install live/cyberpunk-2077/archivexl --yes --path $game --allow-executable engine/tools/scc.exe
modn update live/cyberpunk-2077/archivexl --version 1.28.5 --dry-run --path $game --download --json
modn update live/cyberpunk-2077/archivexl --version 1.28.5 --yes --path $game
modn update live/cyberpunk-2077/archivexl --version 1.28.6 --dry-run --path $game --download --json
modn update live/cyberpunk-2077/archivexl --version 1.28.6 --yes --path $game
modn update live/cyberpunk-2077/archivexl --version 1.28.5 --dry-run --path $game --download --json
modn update live/cyberpunk-2077/archivexl --version 1.28.5 --yes --path $game
modn list --path $game --json |
  Set-Content -LiteralPath "$live\list-after-rollback.json"
```

Expected output:
- The install prints `Installed` for `red4ext`, `redscript` and `archivexl`,
  in that order.
- Each update preview names `installed <version>` and `target <version>`.
- Each update prints `Updated live/cyberpunk-2077/archivexl to <version> (<n> owned files)`.
- The final `list` shows archivexl with `"archive_sha256"` equal to the 1.28.5
  digest.

The first update is the deliberate rollback. The second is a forward update.
The third returns to 1.28.5.

Game-side check: launch the game and reach the main menu.
Confirm that the CET overlay still opens.
Read the newest ArchiveXL log entry under
`red4ext/plugins/ArchiveXL/` and record it.
Quit the game.

#### Step 4: Adopt manual files and clean them up

**Authorize this step.** Modn previews an archive, then claims matching files
that you copy into the game. Adoption changes no game bytes.
Use `no-intro-videos` 0.8, which needs the installed redscript.

```powershell
$adopt = "<path-to-No Intro Videos.zip>"
modn adopt $adopt --dry-run --path $game --json |
  Set-Content -LiteralPath "$live\adopt-preview.json"
```

Expected: `"status": "verification_required"`,
`"target_contents_verified": false`, and a plan that lists every destination.
Check that no destination exists. Stop if one exists.

Copy the archive's files to those destinations only:

```powershell
7z l $adopt
New-Item -ItemType Directory -Force "$live\adopt-staging" | Out-Null
7z x $adopt -o"$live\adopt-staging" -y
$plan = Get-Content "$live\adopt-preview.json" -Raw | ConvertFrom-Json
foreach ($op in $plan.plan.operations) {
  $from = Join-Path "$live\adopt-staging" $op.source
  $to = Join-Path $game $op.destination
  New-Item -ItemType Directory -Force (Split-Path $to) | Out-Null
  Copy-Item -LiteralPath $from -Destination $to
}
modn adopt $adopt --acknowledge --yes --path $game
modn list --path $game --json |
  Set-Content -LiteralPath "$live\list-after-adopt.json"
```

Expected: `Adopted <package-id> (<n> individual owned files; game bytes unchanged)`.
`list` shows the package with `"catalog": null`.
`modn --game cyberpunk-2077 scan --json --path $game` reports those files as
managed.

Game-side check: launch the game and confirm that the intro videos do not play.
Record the observation and quit.

Modify one adopted file to prove the cleanup path. Append one byte to one
adopted file. Read the package ID from `list-after-adopt.json`.
Then run the strict removal and the approved cleanup:

```powershell
modn uninstall <package-id> --dry-run --path $game
modn uninstall <package-id> --yes --path $game
modn uninstall <package-id> --force --dry-run --path $game
modn uninstall <package-id> --force --yes --path $game
```

Expected output:
- The strict uninstall exits nonzero and names the modified file.
  The game bytes stay unchanged.
- The force dry run lists the individual owned files.
- The force removal prints `Uninstalled <package-id>`.
  Any unrecorded file stays.

#### Step 5: Uninstall the remaining managed packages

**Authorize this step.** Modn removes the packages that this run installed.
It removes only files and exclusive directories that it owns.
CET may keep settings in its exclusive directory.
Copy any setting that you want to keep before this step.

```powershell
modn uninstall live/cyberpunk-2077/archivexl --include-deps --dry-run --path $game
modn uninstall live/cyberpunk-2077/archivexl --include-deps --yes --path $game
modn uninstall better-loot-markers --include-deps --dry-run --path $game
modn uninstall better-loot-markers --include-deps --yes --path $game
modn list --path $game
```

Expected output:
- Each dry run prints `Would uninstall, in this order:` and then the packages.
  Record that order.
- Each real command prints `Uninstalled <id>` for every removed package.
- `list` prints `No manager-owned packages installed.`

Game-side check: launch the game and reach the main menu.
Record that the game still starts without the mods.
Quit the game.

#### Step 6: Install and uninstall one REDmod content package

**Authorize this step.** Modn writes one standalone REDmod package into the game.
Pick a real REDmod mod with one canonical layout: `mods/<name>/info.json`, with
optional files under `mods/<name>/archives`, `mods/<name>/scripts` and
`mods/<name>/tweaks`.
Reject an archive with several names, noncanonical casing or a mixed layout.

```powershell
$redmod = "<path-to-REDmod.zip>"
modn inspect $redmod
modn install $redmod --dry-run --path $game --json |
  Set-Content -LiteralPath "$live\redmod-plan.json"
modn install $redmod --yes --path $game
modn list --path $game --json |
  Set-Content -LiteralPath "$live\list-after-redmod.json"
modn --game cyberpunk-2077 scan --json --path $game |
  Set-Content -LiteralPath "$live\scan-after-redmod.json"
modn doctor --path $game
```

Expected output:
- `inspect` lists one `mods/<name>/info.json` and the optional files.
- The preview has `"status": "reviewable"`.
- The install prints `Installed <package-id> (<n> owned files)`.
  Read the ID from `list-after-redmod.json`.
- `list` shows the package with `"catalog": null`.
- `scan` reports the `mods/<name>` folder as unmanaged, because REDmod
  detection is directory-only.
- `doctor` prints `UNMANAGED_CONTENT` for that folder. This is expected.

Game-side check: launch the game and reach the main menu.
Modn does not deploy REDmod and does not verify activation.
Record "REDmod activation not verified".
Do not run the game's deployment tool during this run.
If you do, record it as an external step and record its output.

Uninstall the package:

```powershell
modn uninstall <package-id> --dry-run --path $game
modn uninstall <package-id> --yes --path $game
modn list --path $game
```

Expected: `Uninstalled <package-id>` and `No manager-owned packages installed.`
Files that the game wrote under `mods/<name>` are unmanaged and stay.
Record every remaining path.

#### Step 7: Final cleanup and comparison

**Authorize this step.** Modn forgets the local bucket and the game registration.
Neither action deletes a game file.

```powershell
modn bucket remove live
modn --game cyberpunk-2077 scan --json --path $game |
  Set-Content -LiteralPath "$live\scan-after.json"
modn doctor --path $game
modn game remove cyberpunk-2077 --path $game
```

Expected: `Unregistered cyberpunk-2077 (<game>)`, no managed files in `scan`,
and only runtime files in `doctor`.

Compare the game tree with the baseline:

```powershell
Get-ChildItem -LiteralPath $game -Recurse -File | ForEach-Object {
  $rel = $_.FullName.Substring($game.Length).TrimStart('\')
  "$((Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant())  $rel"
} | Set-Content -LiteralPath "$live\after.sha256"
Compare-Object (Get-Content "$live\baseline.sha256") (Get-Content "$live\after.sha256")
```

Every difference must be a runtime file that the game created or changed.
A missing or changed file that Modn owned is a difference from the fixture
results. Record it and file a bug.

#### What to record for #88

Write these facts into one record:

```text
Cyberpunk 2077 live validation, <date>

Build
- Release: <tag>, commit <sha>
- modn.exe SHA-256: <hash>

Environment
- OS: <caption> <version>, build <build>, <physical|VM>
- Game: Cyberpunk 2077 <version>, <Steam|GOG|Epic>
- Game installation: <path>
- Manager state: <path>

Protection
- Backup: <full copy at ... | SHA-256 manifest at ...>
- User risk acceptance: <yes, restore path <...> | not needed>
- Baseline: <n> unmanaged entries, <n> hashed files

Commands
| Step | Command | Exit | Key output line | Game-side check |
| --- | --- | --- | --- | --- |
| 1 | modn game add ... | 0 | Registered ... | none |
| ... | ... | ... | ... | ... |

Differences from fixture results
- <none>
- <command>: observed <...>; fixtures show <...>

Residue in the game
- <path, or none>
```

Add it as a project memory record with the `project-wiki` skill:

```powershell
uv run --no-project python "$env:USERPROFILE\.agents\skills\project-wiki\scripts\wiki.py" add `
  --kind finding `
  --title "Cyberpunk 2077 live lifecycle validation passed on <date>" `
  --source "https://github.com/modn-org/modn/releases/tag/$tag" `
  --source "https://github.com/modn-org/modn/issues/88" `
  --body-file "$live\record.md"
uv run --no-project python "$env:USERPROFILE\.agents\skills\project-wiki\scripts\wiki.py" lint
```

Commit the record. Keep the raw evidence outside the repository and cite its
local path in the body.
File one bug for each difference from the fixture results:

```powershell
gh issue create --repo modn-org/modn --label bug `
  --title "Live Cyberpunk: <difference>" `
  --body-file "$live\bug.md"
```

Use this bug body:

```text
## Expected
<fixture result, with the gate or test that shows it>

## Actual
<live result, with the command and exit code>

## Reproduction
1. <command>
2. <command>

## Environment
- Release <tag>, modn.exe SHA-256 <hash>
- OS <caption> <version> build <build>
- Game <version>, <store>
- Evidence: <local path>
```

### Windows 10 (#89)

This run closes [issue #89](https://github.com/modn-org/modn/issues/89).
It uses the released `modn.exe` and the verifier binaries from the same commit.
The GitHub release publishes only `modn.exe`, its checksum and `install.ps1`.
The `modn-verifiers-<tag>` workflow artifact holds the desktop binaries.
Download it before it expires, 30 days after the release.

#### Step 1: Record the OS build

**Authorize this step.** Allow read-only OS queries. This step reads no game.

```powershell
[System.Environment]::OSVersion.Version
Get-CimInstance Win32_OperatingSystem | Select-Object Caption, Version, BuildNumber, OSArchitecture
winver
```

Expected: `10.0.19045` for Windows 10 22H2, and a caption that starts
`Microsoft Windows 10`.
Stop if the build is 22000 or higher: that is Windows 11, not #89.

#### Step 2: Get and verify the release and the verifier binaries

**Authorize this step.** Allow downloads into `$live`. This step reads no game.

```powershell
gh release download $tag --repo modn-org/modn --pattern modn.exe --pattern modn.exe.sha256 --dir "$live\release"
$hash = (Get-FileHash "$live\release\modn.exe" -Algorithm SHA256).Hash.ToLowerInvariant()
$expected = ((Get-Content "$live\release\modn.exe.sha256") -split '\s+')[0].ToLowerInvariant()
$hash -eq $expected

$commit = (gh api "repos/modn-org/modn/commits/$tag" --jq .sha)
$run = gh run list --repo modn-org/modn --workflow release.yml --branch $tag --event push --status success --json databaseId,headSha --limit 20 |
  ConvertFrom-Json | Where-Object headSha -eq $commit | Select-Object -First 1
gh run download $run.databaseId --repo modn-org/modn --name "modn-verifiers-$tag" --dir "$live\verifiers"

Get-ChildItem "$live\verifiers\*.exe" | ForEach-Object {
  $want = ((Get-Content "$($_.FullName).sha256") -split '\s+')[0].ToLowerInvariant()
  $got = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
  "{0}: {1}" -f $_.Name, ($got -eq $want)
}
```

Expected: `True` for `modn.exe`, and `True` for each of `modn-mcp.exe`,
`modn-mcp-fault.exe`, `modn-desktop.exe` and `modn-desktop-test-backend.exe`.
Record the tag, commit, workflow run ID and every SHA-256.
Stop if an artifact is missing. Do not use a local build.

#### Step 3: Run the Milestone 1 gate

**Authorize this step.** The gate downloads release assets and runs on a fake
game in a temporary directory. It never touches a real game.

```powershell
git clone https://github.com/modn-org/modn.git "$live\modn"
git -C "$live\modn" checkout $tag
Set-Location "$live\modn"
pwsh -File .github/scripts/milestone1-gate.ps1 -Tag $tag
```

Expected: exit 0, every check line starts `ok:`, and the last line is
`milestone 1 exit gate passed for <tag>`.
The gate uses the released `modn.exe` from the tag, not the checkout binary.
See [Milestone 1 exit gate](#milestone-1-exit-gate) for its checks.
Count the `ok:` lines and record the count.

#### Step 4: Run the desktop verifier with the released binaries

**Authorize this step.** The app opens one window and writes only to unique
`modn-desktop-fixture-*` directories. It never uses a real game.

Prerequisites on the Windows 10 machine: Python 3, 7-Zip on `PATH`,
`agent-desktop`, and an unlocked interactive desktop.
The build doctor needs Rust, CMake, VS 2022 and the Windows SDK.
Skip it for this run: the released drivers do not need a build toolchain.
If the automation state fails its DACL check, set a unique
`AGENT_DESKTOP_HOME` and keep it for every command in this run.

```powershell
agent-desktop skills get desktop > "$live\desktop-skill.json"
agent-desktop skills get platform > "$live\platform-skill.json"
agent-desktop status

python .agents/skills/verify-modn-desktop/scripts/verify.py `
  --app "$live\verifiers\modn-desktop.exe" `
  --backend "$live\release\modn.exe"

python .agents/skills/verify-modn-desktop/scripts/lifecycle.py `
  --app "$live\verifiers\modn-desktop.exe" `
  --backend "$live\verifiers\modn-desktop-test-backend.exe"
```

Expected for each driver:
- exit 0 and `status: passed`
- a positive assertion count and zero failures
- `app_exit: 0` and `fixture_removed: true` in the summary

The v0.3.0 fixture run reported 51 and 8 assertions.
Record both counts from this run.
Read the printed evidence path and keep the summary and screenshots.
Treat that evidence as sensitive local data.

#### What to record for #89

Write these facts into one record:

```text
Windows 10 validation, <date>

Build
- Release: <tag>, commit <sha>
- modn.exe SHA-256: <hash>
- Workflow run: <run id>
- Verifier SHA-256: modn-desktop.exe <hash>, test backend <hash>

Environment
- OS: <caption> <version>, build <build>, <physical|VM>
- Session: <interactive desktop, unlocked>
- Gate game: fake fixture, no real game

Results
| Command | Exit | Key output line |
| --- | --- | --- |
| milestone1-gate.ps1 | 0 | milestone 1 exit gate passed for <tag> |
| verify.py | 0 | status: passed, <n> assertions, 0 failures |
| lifecycle.py | 0 | status: passed, <n> assertions, 0 failures |

Differences from fixture results
- <none>
- <command>: observed <...>; fixtures show <...>
```

Add it as a project memory record with the `project-wiki` skill:

```powershell
uv run --no-project python "$env:USERPROFILE\.agents\skills\project-wiki\scripts\wiki.py" add `
  --kind finding `
  --title "Windows 10 validation passed with the released <tag> binaries on <date>" `
  --source "https://github.com/modn-org/modn/releases/tag/$tag" `
  --source "https://github.com/modn-org/modn/issues/89" `
  --body-file "$live\record-win10.md"
uv run --no-project python "$env:USERPROFILE\.agents\skills\project-wiki\scripts\wiki.py" lint
```

File one bug for each difference, as in
[What to record for #88](#what-to-record-for-88).
