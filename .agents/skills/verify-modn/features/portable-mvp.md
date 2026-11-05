# Portable Windows lifecycle

Run this branch when verifying the Windows alpha lifecycle with a release executable.
Build the binary before running the driver:

```powershell
$env:RUSTFLAGS = '-C target-feature=+crt-static'
cargo build --release --locked -p modn-cli
python .agents/skills/verify-modn/scripts/portable_mvp.py --binary target/release/modn.exe
```

The driver requires Windows, Python 3.10+ and Git for the outside-repository check.
It copies the executable outside the repository and removes Rust tools from the child PATH.
It rejects dynamic Visual C++ runtime imports.
It packs the current official bucket with the pinned bucket repository release packer and bootstraps through explicit `bucket update`.
It uses the bootstrapped first-party Cyberpunk policy without a module override.

The disposable community catalog contains synthetic ZIP payloads and pinned digests.
Its stack contains RED4ext, redscript, CET, ArchiveXL, Codeware, TweakXL, Equipment-EX and a synthetic top package.
The driver checks installed bytes, eight managed packages and all 13 dependency edges.
These fixtures prove policy planning and managed transactions. They do not establish framework or game compatibility.

Update checks cover read-only preview, pin skip, drift rejection before mutation and approved replacement.
The Windows rollback case opens a later shared-location file without delete sharing.
A watcher must observe an earlier target absent before failure.
The driver then checks every original game digest, all database rows and empty transaction staging.
A missing removal witness fails the run. It must not be reported as rollback proof.
Successful replacement must preserve dependency edges and point them to the successor package.

The provider probe selects the pinned public ripgrep 14.1.1 Windows ZIP.
It independently downloads and hashes that ZIP, then starts production provider acquisition with an empty cache.
`install --dry-run --download` must cache exactly those bytes and preserve game files and database rows.
The non-mod ZIP must be layout-blocked after acquisition.
Report transport success separately from install compatibility.
An external provider failure appears as `UNVERIFIED` in the summary; it does not establish live transport coverage.

Each invocation retains `modn-portable-evidence-*` under the OS temporary directory.
Numbered transcripts contain commands, working directories, actual exits, stdout and stderr.
Other evidence contains binary and bucket hashes, database snapshots, rollback samples and the downloaded public ZIP.
`summary.json` lists positive checks, command exits, provider status and fixture cleanup.
Read its error and provider status before reporting acceptance. A zero exit alone is insufficient.

The driver uninstalls managed content and checks that original unrelated bytes remain.
It verifies idle recovery and removes only its own `modn-portable-fixture-*` directory.
Evidence remains available after cleanup. No real game, release publication or credential is needed.
