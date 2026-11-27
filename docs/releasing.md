# Releasing

The `Release` workflow publishes only when a maintainer pushes a `v*` tag. The tag
must equal the `modn-cli` version, for example `v0.3.0`. The workflow builds
`modn.exe` on Windows and smoke-tests it on a second, clean runner. The smoke
test runs from an empty directory, with no repository and no `cargo` or `rustc`
on `PATH`. It checks the checksum, the absence of C runtime DLL imports,
`search`, `install --dry-run` for a catalog mod and for a local archive, and
`doctor` on a fake game. It also checks that the fake game's files and their
contents are unchanged. The smoke test needs a contract v2 `main` bucket. It
installs the pinned `buckets/main` submodule, packed with the bucket
repository's v2 packer and signed with an ephemeral test key. **Precondition
for a Modn release:** `modn-org/main` must publish a contract v2 bucket, and
this repository must pin that commit. Since Modn 0.7.0 the submodule pins the
contract v2 bucket of `main` release `v2026.11.24`. A Modn release and the restructured, signed `modn-org/main` release
ship together. Only then does the workflow publish the GitHub release with
`modn.exe`, its checksum and `install.ps1`. Bucket releases come from `modn-org/main`. Each bucket release
must include `main-bucket.zip.minisig` and `main-bucket.requires.json`. The bucket
repository's tag release signs the v2 catalog with the key built into Modn;
`v2026.11.08` was the first signed v1 bucket release. A version with a pre-release
suffix, such as `0.2.0-alpha.1`, becomes a GitHub pre-release. Pull requests
that change the workflow or the smoke script run the build and smoke jobs, but
they never publish.

## Versioning

Modn uses Cargo's 0.x semantic versioning convention.
The version describes changes to the public API.

The public API is:

- the CLI commands and their options
- the `--json` output
- the state files, with the state schema version
- the bucket contract, with `schema_version` in `bucket.json`
- the game-module host API, with `api_version` in `module.json`
- the theme API, with `theme_api` in `theme.json`

Rules before 1.0:

- A minor release (`0.y.0`) may break the public API.
- A patch release (`0.y.z`) must not break the public API.
- A release that raises the state schema, the bucket contract, the host API or the theme API is a minor release. Its release notes name each raised version.
- A release that only fixes defects is a patch release.

### Bucket release requirements

Every `modn-org/main` release must upload `main-bucket.requires.json` beside
`main-bucket.zip` and `main-bucket.zip.minisig`. The file has this shape:

```json
{"schema_version":2,"min_modn_version":"0.7.0"}
```

Set `schema_version` to the `bucket.json` contract in the packed ZIP. It is `2`
for the current contract. Set `min_modn_version` to the oldest Modn release that
can read the packed bucket. Check the bucket contract, package manifests, game
modules and host API versions. Use the Cargo version without a leading `v`.
When the bucket needs a new Modn feature, publish that Modn release first and
name it here. Do not lower the value to make the bucket available to an older
build.

The release job must generate this file from the packed tree and validate both
values before upload. A missing or malformed file must fail the release. Modn
skips a newer release when its schema is unsupported or its minimum Modn version
is above the running build. A release without this file remains readable for
legacy compatibility, but the install checks still validate its contents.

Modn 1.0 needs a stable bucket contract v2, host API 3 and theme API 1.

### Numbered catalog releases

Every `main` release carries a signed `sequence.json` with a number higher than
any earlier release. The release job signs the packed tree with Modn's authoring
command before it packs the ZIP:

```sh
modn catalog sign . --key ../main-keys/main.key --sequence next --expires 2027-12-31T00:00:00Z
```

`--sequence next` writes the number in the existing `sequence.json` plus one, or
1 when the file is absent. `--expires` is optional. When given, it must be a UTC
timestamp in `YYYY-MM-DDTHH:MM:SSZ` form and in the future. Commit
`sequence.json` with the release, because the catalog digest covers it and
`catalog.sig` signs that digest. Pack the ZIP only after the signing step, so
`main-bucket.zip` contains the same `sequence.json`.

Users keep accepting unnumbered catalogs. Once a user accepted a numbered
release, a later release with a lower number, or with no number, is refused as a
rollback. The first numbered release therefore works as a normal upgrade, and
every release after it must keep increasing the number. An expired release only
warns; it never blocks an update.

If a release published a number by mistake, never reuse that number. Publish a
higher number, for example with `--sequence next`. A user who already accepted
the wrong number refuses a lower one; that user can delete
`<profile>/buckets/main.json` by hand and run `modn bucket update` to accept the
older number deliberately. Keep the file deletion as the documented recovery
step; do not add a flag that disables rollback protection.

### Release checklist

1. Compare the changes since the last tag with the public API above.
2. Choose the version by the rules above. Set it in the workspace `Cargo.toml` and update `Cargo.lock`.
3. Write the release notes. Name each raised state schema, bucket contract, host API or theme API version.
4. Publish the matching `modn-org/main` bucket release first when the Modn release needs it. Include `main-bucket.requires.json` with the values from [Bucket release requirements](#bucket-release-requirements) and a higher signed sequence number from [Numbered catalog releases](#numbered-catalog-releases). See the precondition above.
5. Push the `v<version>` tag.

Run the same smoke test locally on a built executable:

```powershell
$env:RUSTFLAGS = '-C target-feature=+crt-static'
cargo build --release --locked -p modn-cli
$hash = (Get-FileHash target/release/modn.exe -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText("$PWD/target/release/modn.exe.sha256", "$hash *modn.exe`n")
python buckets/main/.github/scripts/pack.py buckets/main target/smoke/main-bucket.zip
Expand-Archive -LiteralPath target/smoke/main-bucket.zip -DestinationPath target/smoke-bucket -Force
cargo run --quiet --locked -p modn-core --example sign_catalog -- target/smoke-bucket `
  target/smoke/main-bucket.zip.minisig target/smoke/main-bucket.pub
cargo build --release --locked -p modn-cli --features modn-core/test-main-key --target-dir target/smoke
Copy-Item target/smoke/release/modn.exe target/smoke/modn-test.exe
pwsh -File .github/scripts/release-smoke.ps1 -Binary target/release/modn.exe `
  -Checksum target/release/modn.exe.sha256 -MainBucket target/smoke/main-bucket.zip `
  -BucketBinary target/smoke/modn-test.exe -PublicKey target/smoke/main-bucket.pub
```

`modn bucket update` verifies a signature, so the packed ZIP needs its matching
`main-bucket.zip.minisig` beside it.
`sign_catalog` generates an ephemeral key in memory, writes the signature and
public key, and prints the key ID. It never writes the secret key.
The `-BucketBinary` build enables `test-main-key` so the smoke test can accept
that ephemeral key for `bucket update`. This binary is for the test only, and
the release publishes no copy of it.
`-PublicKey` names the key that signed the ZIP, and the script then requires
`main-bucket.zip.minisig` beside it.

## Rehash main for contract v2

`main` lives in the separate `modn-org/main` repository.
A contract v1 checkout keeps manifests in `games/<game-id>/` and pins each archive with SHA-256.
The maintainer script converts that checkout into a contract v2 tree.
It downloads every archive, verifies it against its current SHA-256 pin, writes its BLAKE3 pin with `modn catalog hash`, and keeps the old source as the `sources` list.
It writes nothing when any archive does not match or any record is invalid.

Preconditions:

- A clean `modn-org/main` checkout at the commit to rehash. Work on a copy outside the submodule; the script never changes its input.
- A Modn binary with `modn catalog hash` (#121). Pass it with `--modn`, or put `modn` on `PATH`.
- Every archive that a `manual` or `nexus` record names, with the record's exact file name, in one directory.
  The script downloads `https` and `github-release` archives. It refuses an `http` URL and any `modn-org` host, so `sources` lists only origins that the author publishes.

Run the conversion:

```powershell
python .github/scripts/rehash_main.py C:\path\to\main-v1 `
  --out C:\path\to\main-v2 --archive-dir C:\path\to\archives `
  --modn C:\path\to\modn.exe
```

`--out` must be missing or empty. The script stages the whole tree beside it and renames the finished tree into place.
On a mismatch it prints every failing package with its reason, its expected SHA-256 and its actual SHA-256, and exits nonzero.
`--out` keeps no files from the failed run, and no staging directory remains.

To verify every SHA-256 pin without writing a tree, add `--check` and omit `--out`:

```powershell
python .github/scripts/rehash_main.py C:\path\to\main-v1 `
  --archive-dir C:\path\to\archives --check
```

Review the diff before you commit:

- `bucket.json` has `schema_version: 2`.
- Records are under `mods/<game-id>/`. An archive record has `blake3`, `upstream_sha256` and one `sources` entry, which is the old `source` object. It has `redistributable: false`, because a v1 record carries no author consent.
- A git-only record keeps its commit pin. It has no `blake3` and no added `archive_format`.
- `games/<game-id>/` holds only `module.json` and `module.lua`, copied unchanged.
- `PROVENANCE.md` has the "Rehash to contract v2" section with every old SHA-256 and new BLAKE3.

Commit the tree to `modn-org/main`, then sign and verify it at the repository root:

```sh
modn catalog sign . --key ../main-keys/main.key --sequence next
modn catalog verify . --key catalog.pub
```

Then run the bucket release checks in [Before any live step](#before-any-live-step).
The release must be a contract v2 bucket, include `main-bucket.zip.minisig`, and pass the `identical` or `ahead` comparison with the Modn pin.
The script does not sign, tag, upload or publish anything.
Publishing the restructured `main` release together with the Modn release is a maintainer action that this script does not perform.

## Milestone 1 exit gate

Both milestone gate scripts target contract v1 and stay unchanged as closed-milestone evidence; do not run them against a v2 bucket.

The closed gate ran after a release against its published assets:

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

This gate closed Milestone 2, and its results stay as evidence.
`.github/scripts/milestone2-gate.ps1` and `milestone1-gate.ps1` target contract
v1 and cannot run against a v2 bucket. Do not run them for a current release.
The results below are the record of the closed milestone.

These checks replace the gate for current releases:

```powershell
python .agents/skills/verify-modn/scripts/verify.py --build
cargo test --workspace --locked
```

`verify.py --build` builds the CLI and checks the bucket, module, install and
undo flows against disposable fixtures.
The cargo tests cover the transaction, ownership and module suites.
`python .agents/skills/verify-modn/scripts/portable_mvp.py` checks the portable
release build; it needs `target/release/modn` and the `test-main-key` smoke
build.

The closed gate ran these checks against disposable game installations:

1. Sync a local Git catalog twice and resolve its package.
2. Install pinned redscript bytes through a `github-release` source, then uninstall them.
3. Refuse incomplete adoption approvals, then adopt matching existing bytes with explicit approval.
4. Refuse strict removal of modified owned bytes, then remove them with approved cleanup.
5. Install and uninstall canonical REDmod content without running a game or deployment tool.
6. Refuse a fixture main bucket that requires Modn 99.0.0, and preserve the previous bucket.
7. Run the existing MCP verifier and the desktop verifier.

The Release build job builds four verifier binaries from the same commit as `modn.exe`.
All five binaries use release optimization and a static C runtime.
The workflow stores the verifiers and their SHA-256 files in `modn-verifiers-<tag>` for 30 days.
Pull request runs also build and upload the verifiers, but never publish a release.
The GitHub release contains only `modn.exe`, its checksum, and `install.ps1`.

Tag mode requires an authenticated `gh` CLI with Actions read access.
It selects a successful Release workflow run for the tag and checks its commit against the tag commit.
It downloads that run's verifier artifact and verifies all four SHA-256 files before execution.
It does not use local build directories.
Tag mode checks the artifact before it expires.
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

Install Python 3, Git, 7-Zip and Node.js before running the gate.
Desktop verification requires Windows 10 or newer and an unlocked interactive desktop.
The desktop build doctor requires Rust and build tools, so the gate does not run it.
The desktop verifier drives prepared binaries; it does not compile them itself.
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
The closed gate passed against published assets after tagging for v0.3.0 and v0.5.0.
Those runs are its record and the evidence for issue #87.

### Result for v0.3.0

On 2026-11-05, `milestone2-gate.ps1 -Tag v0.3.0` passed against the published v0.3.0 assets.
It reported 15 checks and 0 failures.
The Milestone 1 gate passed for v0.3.0, including state created by v0.1.0.
The MCP verifier passed 69 checks.
The desktop verifiers passed 51 and 8 assertions.
The refusal check printed `module cyberpunk-2077 needs Modn 99.0.0, you have 0.3.0`.
The original Modn roots were unchanged.
The released `modn.exe` SHA-256 is `2dade4e26e3db81ebf2dd62f0fc38a487184fc6968139dce43f1fd4bfa90eadb`.

### Result for v0.5.0

On 2026-11-08, `milestone2-gate.ps1 -Tag v0.5.0` passed against the published v0.5.0 assets.
It reported 16 checks and 0 failures.
The released install script verified the signed `main` release `v2026.11.08`: `Verified main bucket signed by 0E250AD52A3BA2AA.`
The Milestone 1 gate passed for v0.5.0, including state created by v0.1.0.
The Git catalog check synced a signed catalog and pinned its signer key.
The MCP verifier passed 69 checks.
The desktop verifiers passed 51 and 8 assertions.
The refusal check printed `module cyberpunk-2077 needs Modn 99.0.0, you have 0.5.0`.
The original Modn roots were unchanged.
The released `modn.exe` SHA-256 is `9dd9b30fce82f5529558d9b755fd0a4a2e37724a14083c7ac134d3c0a9b73ada`.

The first two runs found three gate defects, fixed before the passing run:

- The Git catalog fixture was unsigned, and Modn 0.5.0 refuses unsigned catalogs.
- The gate parsed stdout and stderr together as JSON. The new signer fingerprint notice goes to stderr.
- `lifecycle.py` decoded `tasklist` output as UTF-8, which fails on a non-English Windows console.

## Real-game validation

The gates above use disposable fixtures and prove no real-game compatibility or REDmod activation.
The Milestone 3 exit gate needs live evidence from Windows 11.
Run [Cyberpunk 2077 (#88)](#cyberpunk-2077-88) and [Windows 11 (#89)](#windows-11-89) on Windows 11.
Windows 10 stays unverified: the maintainer has no Windows 10 machine.
CI runs on Windows Server and runs no desktop verifier, so it does not cover either client OS.
[Balatro](#balatro) covers the native injector, the Git source and the content root.

### Before any live step

Run every command in PowerShell. Set these values once and record them:

```powershell
$tag = (gh release view --repo modn-org/modn --json tagName --jq .tagName)
$live = "$env:USERPROFILE\modn-live"
$game = "<game-root>"
New-Item -ItemType Directory -Force $live | Out-Null
```

**Release.** Use the latest published release for the whole run, `v0.5.0` at the time of writing.
Do not mix binaries from different tags. Download and verify it:

```powershell
gh release download $tag --repo modn-org/modn --pattern modn.exe --pattern modn.exe.sha256 --dir "$live\release"
(Get-FileHash "$live\release\modn.exe" -Algorithm SHA256).Hash
Get-Content "$live\release\modn.exe.sha256"
gh api "repos/modn-org/modn/commits/$tag" --jq .sha
```

Expect the two hashes to match, in any case, and a full commit SHA.
Record the tag, the commit and the SHA-256. Install Python 3, Git, `gh` and 7-Zip on `PATH`.

**Bucket release.** `modn bucket update` installs the newest `modn-org/main` release this build supports, not the bucket commit that `$tag` pins.
The release must use contract v2: `bucket.json` with `schema_version: 2`, package manifests under `mods/<game-id>/` and game modules under `games/<game-id>/`.
This Modn refuses a contract v1 bucket.
The release must include `main-bucket.zip.minisig`, signed by the key built into Modn, and `main-bucket.requires.json` with `min_modn_version` no higher than this Modn version.
Since Modn 0.7.0, this repository's submodule pins a contract v2 commit; a Modn release and the bucket release that contains its pin ship together.
Confirm that the bucket release contains that commit:

```powershell
$pin = gh api "repos/modn-org/modn/contents/buckets/main?ref=$tag" --jq .sha
$bucket = gh release view --repo modn-org/main --json tagName --jq .tagName
gh api "repos/modn-org/main/compare/$pin...$bucket" --jq .status
```

Expect `identical` or `ahead`. Record the pin and the bucket tag.
On `behind` or `diverged`, stop: the bucket release lacks module changes that this Modn release expects.
Publish a bucket release from `modn-org/main`, then start again.

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

**Clean installation.** Do this before the backup, so the baseline shows the game as the run starts:

```powershell
modn list
$present = 'bin\x64\plugins\cyber_engine_tweaks', 'bin\x64\version.dll', 'bin\x64\winmm.dll',
  'red4ext', 'r6\scripts', 'engine\tools\scc.exe' |
  Where-Object { Test-Path -LiteralPath (Join-Path $game $_) }
if ($present) { throw "Stop: framework files present: $($present -join ', ')" }
```

Expect `No manager-owned packages installed.` and no error.
Otherwise, clean the installation first, then repeat this check.
Remove Modn-owned packages with `modn uninstall`, and other mods by their own instructions.
If the game folder changes after the backup and before step 1, take the backup again.

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
$main = "<data dir>\buckets\main"  # the contract v2 official bucket copy; see buckets.md
New-Item -ItemType Directory -Force "$bucket\mods\cyberpunk-2077" | Out-Null
'{"schema_version":2,"name":"live","title":"Live validation"}' | Set-Content "$bucket\bucket.json"
'archivexl','red4ext','redscript' | ForEach-Object {
  Copy-Item "$main\mods\cyberpunk-2077\$_.json" "$bucket\mods\cyberpunk-2077\"
}
$url = 'https://github.com/psiberx/cp2077-archive-xl/releases/download/v1.28.5/ArchiveXL-1.28.5.zip'
Invoke-WebRequest $url -OutFile "$live\ArchiveXL-1.28.5.zip"
$old = Get-Content "$bucket\mods\cyberpunk-2077\archivexl.json" -Raw | ConvertFrom-Json
$old.version = '1.28.5'
$old.sources = @(@{kind = 'https'; url = $url})
$old.blake3 = (modn --json inspect "$live\ArchiveXL-1.28.5.zip" | ConvertFrom-Json).blake3
$old | ConvertTo-Json -Depth 5 | Set-Content "$bucket\mods\cyberpunk-2077\archivexl@1.28.5.json"
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
$created = @()
foreach ($op in $plan.plan.operations) {
  $to = Join-Path $game $op.destination
  for ($dir = Split-Path $to; -not (Test-Path -LiteralPath $dir); $dir = Split-Path $dir) { $created += $dir }
  New-Item -ItemType Directory -Force (Split-Path $to) | Out-Null
  Copy-Item -LiteralPath (Join-Path "$live\adopt-staging" $op.source) -Destination $to
}
$created | Select-Object -Unique | Set-Content "$live\adopt-created-folders.txt"
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

Modn does not own the folders that the copy created. Remove each one that is now empty:

```powershell
Get-Content "$live\adopt-created-folders.txt" | Sort-Object { $_.Length } -Descending |
  Where-Object { (Test-Path -LiteralPath $_) -and -not (Get-ChildItem -LiteralPath $_ -Force) } |
  ForEach-Object { Remove-Item -LiteralPath $_; "Removed empty $_" }
```

Expect one `Removed empty` line per created folder, such as `r6\scripts`.
A folder that still holds a file stays. Record it.

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
Use Modn 0.5.0 or later, from [Before any live step](#before-any-live-step). Record its commit and SHA-256.
Run `modn bucket update` first. The `main` bucket release must use contract v2 and contain the two Balatro packages.
Every command and flag below exists in the built CLI. Check one with `modn <command> --help`.

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
modn --game balatro install main/balatro/steamodded --yes --path $game
modn --game balatro list --path $game
```

Expect `Installed main/balatro/steamodded (<n> owned files)`.
Expect no `Installed` line for Lovely, because that dependency is ready.
Expect `list` to show both packages. Record the package ID and `n`.
Do not pass `--download` on the install. It only prepares a dry run, and `install --yes` refuses it.
The install reuses the Git tree that the preview fetched.

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
Copy the evidence you cite to a durable path, then remove `$live`.
If a check failed, keep `$live` for the bug report.
Do not delete `%APPDATA%\Balatro`, `Mods` or any unmanaged file during cleanup.

#### What to record for Balatro

```text
Balatro live validation, <date>
- Modn <tag or build>, commit <sha>, modn.exe SHA-256 <hash>, state <path>
- main bucket <digest or source>; search <two package lines | missing>
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

### Windows 11 (#89)

This run closes [issue #89](https://github.com/modn-org/modn/issues/89) with the released `modn.exe`
from [Before any live step](#before-any-live-step) and verifier binaries from the same commit.
The `modn-verifiers-<tag>` workflow artifact holds the desktop binaries.
Download it before it expires, 30 days after the release.

#### Step 1: Confirm the OS build

**Authorize this step.** Allow read-only OS queries.

Run the OS version commands in [Before any live step](#before-any-live-step).
Expect build 22000 or higher and a `Microsoft Windows 11` caption.
Stop if the build is lower than 22000: that is Windows 10, which #89 does not cover.

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

#### Step 3: Run the portable verifier

**Authorize this step.** The verifier builds disposable contract v2 fixtures and a fake game in a temporary directory.

Prerequisites: a checkout of this repository at the release tag, Python 3, and a Rust toolchain with `cargo`.
The verifier runs `cargo run --quiet --locked -p modn-core --example sign_catalog` to sign its disposable main bucket, so `cargo` must be on `PATH`.
The verifier tests the published executable: pass the `modn.exe` that you downloaded and checked in Step 1 (`$live\release\modn.exe`) through `--binary`. Do not pass a local build there.
The checkout and `cargo` serve only the test-key signing prerequisite: build the smoke copy with the test main key at that checkout with `$env:RUSTFLAGS = '-C target-feature=+crt-static'` and `cargo build --release --locked -p modn-cli --features modn-core/test-main-key --target-dir target/smoke` (the commands near the top of this page).
The verifier reads that smoke build through `--bucket-binary`; its default is `target/smoke/release/modn.exe`.
The verifier fails its static C runtime check when a binary imports a dynamic Visual C++ runtime; the published release binary links the runtime statically.

The Milestone 1 gate targets contract v1 and cannot pass against a contract v2 release. Do not run it.
Run the portable verifier against the released `modn.exe` instead.
Run `python .agents/skills/verify-modn/scripts/portable_mvp.py --help` for its arguments, and expect exit 0 and no failed check.
Then run the verifier with the two binaries:

```powershell
python .agents/skills/verify-modn/scripts/portable_mvp.py `
  --binary "$live\release\modn.exe" `
  --bucket-binary target/smoke/release/modn.exe
```

See [Milestone 1 exit gate](#milestone-1-exit-gate) for the closed v1 evidence.
Count the `ok:` lines and record the count.

#### Step 4: Run the desktop verifier

**Authorize this step.** The app opens one window and writes only to unique `modn-desktop-fixture-*` directories.

Prerequisites: Windows 10 or later, WebView2, an unlocked interactive desktop, Python 3, `uv` and 7-Zip.
The journeys need no Rust compiler; the next command uses `cargo` once for the verifier CLI.
Skip the desktop build doctor.

Use the `modn-desktop.exe` and `modn-desktop-test-backend.exe` verifier binaries from Step 2.
Do not pass the published `modn.exe` as `--cli`.
A release build compiles out the `MODN_STORE_LIBRARIES` and `MODN_APPDATA` test overrides (`crates/modn-core/src/stores.rs:143-155`, `crates/modn-core/src/content_root.rs:262-274`).
Without them, detection reads the real Steam library and the content-root journeys write outside the fixture.
The verifier exits with status 2 before any journey when the `--cli` binary lacks them.
Build a verifier CLI from the release tag instead:

```powershell
cargo build --release --locked -p modn-cli --features modn-core/test-instruction-budget
```

Run the verifier against the three prepared binaries:

```powershell
python .agents/skills/verify-modn-desktop/scripts/verify.py `
  --app "$live\verifiers\modn-desktop.exe" `
  --test-backend "$live\verifiers\modn-desktop-test-backend.exe" `
  --cli target/release/modn.exe
```

Expect exit 0 and a JSON summary with `"status": "passed"`, `"journeys": 11`, `"passed": 11` and `"failed": 0`.
Each journey prints an `evidence: <path>` line, and each result in the summary repeats that path in `evidence`.
Record those paths; the run keeps evidence without `--keep`.
The v0.3.0 fixture run reported 51 and 8 assertions for the older drivers. Record the new counts.

#### What to record for #89

Write these facts into one record:

```text
Windows 11 validation, <date>
- Release <tag>, commit <sha>, modn.exe SHA-256 <hash>, workflow run <id>
- Verifier SHA-256: modn-desktop.exe <hash>, test backend <hash>
- OS <caption> <version>, build <build>, <physical|VM>; gate game <fake fixture>

| Command | Exit | Key output line |
| --- | --- | --- |
| portable_mvp.py | 0 | its final summary line, no failed check |
| verify.py | 0 | status: passed, 11 journeys, <n> assertions, 0 failures |

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
