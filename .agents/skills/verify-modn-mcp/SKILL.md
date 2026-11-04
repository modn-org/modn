---
name: verify-modn-mcp
description: Verify Modn MCP stdio framing, scoped arguments, previews, approved writes, and backend limits with disposable real CLI fixtures.
---

# Verify Modn MCP

Use this skill after changes to `apps/modn-mcp` or its backend contract.
Read [features/README.md](features/README.md) for the feature map.
Use native execution so retained evidence survives subprocess cleanup.

## Launch and doctor

From the repository root, build the real companion and CLI:

```powershell
cargo build --locked -p modn-cli -p modn-mcp
rustc apps/modn-mcp/tests/fixtures/backend.rs -o "$env:TEMP/modn-mcp-fault.exe"
target/debug/modn-mcp.exe --help
```

Check each command's exit status. The help output must identify the scoped stdio companion.
The driver uses a five-second real CLI deadline and a one-second fault backend deadline.
It starts each process itself.
It needs Python 3 and the Rust toolchain. It uses Python's standard library only.

## Drive and evidence

```powershell
python .agents/skills/verify-modn-mcp/verify.py --server target/debug/modn-mcp.exe --backend target/debug/modn.exe --fault-backend "$env:TEMP/modn-mcp-fault.exe" --evidence "$env:TEMP/modn-mcp-evidence"
```

The driver creates a unique temporary fixture directory.
It copies the official bucket from this repository and registers a fake Cyberpunk installation through the real CLI.
It exchanges MCP messages through pipes, then checks file hashes and CLI state independently.
Read the driver's exit status. Require `failures: 0` and a positive `checks` count.
Keep `summary.json` and `protocol.json` in the named evidence directory.
The driver also retains incremental `responses.jsonl` and `progress.json` if an assertion fails.
Failed runs report an assertion and keep the terminal log when redirected by the caller.
Never substitute a real game installation.

Run repository gates after the journey:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
```

## Cleanup

The driver closes each server's stdin and waits for exit.
It deletes only its own temporary fixtures.
Evidence remains in the requested directory.
If a run fails before cleanup, stop only process IDs created by that run.
Keep failure logs and report the failed check before retrying.
On other platforms, replace `.exe` paths with the corresponding native executable paths.
Windows is the initially verified platform.
