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
  -Binary target/release/modn.exe -Checksum target/release/modn.exe.sha256
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

The interface verifiers require extra binaries. The Release workflow publishes
only the CLI, its checksum, and the installer.
Build these prerequisites on a development machine before removing Rust from PATH:

```powershell
cargo build --workspace --all-features --locked
rustc apps/modn-mcp/tests/fixtures/backend.rs -o target/debug/modn-mcp-fault.exe
```

The gate reads these binaries from `target/debug` by default.
Use `-VerifierBinaries <directory>` to select a prepared directory on another machine.
That directory must contain `modn-mcp.exe`, `modn-mcp-fault.exe`,
`modn-desktop.exe`, and `modn-desktop-test-backend.exe`.
Use binaries built from the same revision as the selected CLI.
The gate does not build them or certify their provenance.

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
All game writes use fixtures beneath its printed temporary directory.
It retains logs, fixture files, and `summary.json` there for inspection.
It compares the original Modn roots before and after the run.
Each check prints `PASS` or `FAIL`. Any failure produces a nonzero exit.
These fixture checks do not establish real-game compatibility or REDmod activation.
The gate must pass against published assets after tagging; local proof alone does not close issue #87.
