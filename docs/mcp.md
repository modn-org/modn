# Local Modn MCP companion

`modn-mcp` exposes scoped Modn CLI operations over local standard input and output.
It opens no network listener. Build both binaries from the same revision:

```powershell
cargo build --locked -p modn-cli -p modn-mcp
```

Create a dedicated manager directory named `modn` outside every game directory.
Provision its official bucket and game registrations with the normal CLI first.
Use an existing game installation only after you review the approved filesystem roots.
Use disposable fixtures for verification.

The state directory name aligns the CLI's explicit state selection with its platform data-directory convention.
For example, `C:\ModnMcp\modn` uses `C:\ModnMcp` as the backend's `LOCALAPPDATA` and `XDG_STATE_HOME`.
Set this environment when you provision the dedicated CLI state.
The server clears inherited backend environment variables, including developer module overrides.
It passes `SystemRoot` on Windows for operating-system runtime support.

```powershell
$env:LOCALAPPDATA = 'C:\ModnMcp'
target/debug/modn.exe --state-dir C:\ModnMcp\modn bucket update
target/debug/modn.exe --state-dir C:\ModnMcp\modn game add cyberpunk-2077 --path C:\Games\Cyberpunk2077
target/debug/modn-mcp.exe --backend C:\src\modn\target\debug\modn.exe --state-dir C:\ModnMcp\modn --allow-root C:\Games --allow-root C:\ModArchives
```

Provisioning can download the official bucket. MCP preview tools never download.
The first MVP accepts only the official local bucket and no registered third-party modules.
It refuses nonempty `buckets/registrations` and `game-modules` directories.
Keep all registered game installations within the configured roots.
Use dedicated state that contains no unrelated registrations or sensitive data.

## Client configuration

Configure a client that supports local MCP stdio servers:

```json
{
  "mcpServers": {
    "modn": {
      "command": "C:\\src\\modn\\target\\debug\\modn-mcp.exe",
      "args": [
        "--backend", "C:\\src\\modn\\target\\debug\\modn.exe",
        "--state-dir", "C:\\ModnMcp\\modn",
        "--allow-root", "C:\\Games",
        "--allow-root", "C:\\ModArchives"
      ]
    }
  }
}
```

Use absolute executable paths. The backend and manager directory remain fixed for the server lifetime.
Client tool arguments cannot replace either value.
Select one to eight existing directory roots at startup.
Archive and game arguments must be absolute existing paths beneath those canonical roots.
Parent traversal and redirected input components are refused.
Game directories must remain separate from the manager directory.
The manager root and its tree are checked for redirects before backend calls.
The tree has a 20,000-entry limit.
The CLI retains destination, ownership, transaction, rollback, and recovery checks.

## Tool contracts

All inputs are JSON objects. Unknown input fields fail validation.
All tools return MCP text content containing the CLI's JSON result.
Backend failures return `isError: true` with a bounded diagnostic category.
Argument and scope errors use JSON-RPC error `-32602`.

| Tool | Required inputs | Behavior |
| --- | --- | --- |
| `catalog_search` | None; optional `query` | Searches the local official bucket. |
| `registered_games` | None | Lists registered game installations. |
| `inspect_archive` | `archive` | Inspects ZIP, JAR, or 7z metadata through the CLI. |
| `preview_install` | `archive`, `game_path`, `game_id` | Runs local archive installation with `--dry-run`. |
| `preview_uninstall` | `package`, `game_path`, `game_id` | Runs managed package removal with `--dry-run`. |
| `install_archive` | `archive`, `game_path`, `game_id`, `approved: true` | Available only with server `--allow-writes`; invokes CLI `--yes`. |
| `uninstall_package` | `package`, `game_path`, `game_id`, `approved: true` | Available only with server `--allow-writes`; invokes CLI `--yes`. |

`game_id` is one game identifier. `package` is a managed package ID or qualified catalog selector.
Search queries have a 200-byte limit and cannot start with `-` or contain control characters.
Input paths have a 4,096-byte limit. Identifiers have a 200-byte limit.
The server passes literal process arguments and never starts a shell.
Clients cannot invoke arbitrary CLI commands, recovery, registration, downloads, or executable approvals.
The CLI can refuse packages that need executable permission. This companion does not grant that permission.
The client must obtain user approval before setting `approved: true`.
The approval value authorizes that request; it does not bind a previous preview to an unchanged archive.
Review the archive and close other managers before an approved write.

Tool annotations deliberately use `readOnlyHint: false`, `destructiveHint: true`, and `idempotentHint: false`.
These conservative hints avoid promising immutable manager state across backend versions.
`openWorldHint: false` describes the fixed local tool surface.
Annotations are hints, not an isolation boundary.

## Protocol and limits

The companion implements MCP `2025-06-18` over newline-delimited UTF-8 JSON-RPC 2.0.
Clients initialize, send `notifications/initialized`, then use `ping`, `tools/list`, and `tools/call`.
String and integer request IDs are preserved. Null and fractional IDs are refused.
The server responds with its supported protocol version during version negotiation.
Clients must disconnect if they cannot support that version.
Unknown notifications produce no response. Notifications never execute tools.
Unknown methods return `-32601`; malformed JSON returns `-32700`.
Malformed envelopes and oversized messages return `-32600` with a null ID.
Batches, resources, prompts, sampling, and pagination are outside this MVP.
Standard MCP shutdown closes stdin. There is no `shutdown` request.

## Progress and cancellation

A `tools/call` request may carry `params._meta.progressToken`, a string or an
integer. While the call runs, the server sends one
`notifications/progress` message for each CLI phase: resolve, each download,
extract, stage, commit, and result.
Each notification repeats the token and has a strictly increasing `progress`.
A download also carries `total` when the CLI states the expected size.
The `message` field is a short phase label, such as `committing <package>`.
A call without a token sends no progress notification and otherwise behaves as before.

A client cancels with `notifications/cancelled` and `params.requestId` set to
the running request ID.
The server writes one `cancel` line to the CLI and keeps serving later requests.
A cancellation for a request still waiting in the queue removes that request.
The removed request never starts and sends no response.
The server keeps reading input while a call runs, including when the bounded
queue is full, so a cancellation for the active call is never delayed by queued
requests.
The CLI's [commit point](transactions.md) decides the result.
Before it, the call stops and the server sends no response for that request;
game files, manager state and ownership stay unchanged.
After it, the CLI ignores the request and the call completes normally, with its
usual response.
The server tracks the commit phase from the CLI events, also when no progress
token was requested.
The server drains every queued progress notification before it writes the
matching `tools/call` response, so the final `result` notification always
precedes that response.
The client must read progress notifications before the matching response.

The CLI keeps its normal deadline, byte limits and serial execution.
Cancellation is cooperative; a backend that does not report events is only
stopped by its deadline.

Each input line is limited to 65,536 bytes, including its newline.
The server discards an oversized line and resumes at the next newline.
Backend standard output is limited to 262,144 bytes.
A backend that exceeds it is killed at once and the call fails with the
output-limit error.
Diagnostic stderr lines are limited to 262,144 bytes in total.
Valid `--progress json` event lines do not count toward that diagnostic limit,
so a large download does not exhaust it.
Serialized responses are limited to 524,288 bytes before the newline.
Backend failures never echo complete stderr or environment variables.
The default backend deadline is 30 seconds. `--timeout-seconds` accepts 1 through 300.
Calls execute serially. A bounded queue holds four requests that arrive during a call.
The server keeps reading input while the queue is full.
An ordinary request above the limit receives JSON-RPC error `-32000` and does
not run; cancellation notifications are handled at any time.
EOF processes a final unterminated line, then drains queued requests.
It waits at most the configured deadline for each backend call.
Timed-out or oversized backend processes are killed and reaped.
No backend remains after normal server exit. Clients can terminate the server if its output consumer stalls.
stdout contains protocol responses and progress notifications only. Startup diagnostics use stderr.

## Implementation choice and limits

The official Rust SDK is `rmcp`: <https://github.com/modelcontextprotocol/rust-sdk>.
This MVP uses a small explicit dispatcher to keep framing, queue, error, and byte limits visible and testable.
It does not claim full SDK feature coverage.
The protocol follows the authoritative [stdio transport](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports),
[lifecycle](https://modelcontextprotocol.io/specification/2025-06-18/basic/lifecycle),
and [tools](https://modelcontextprotocol.io/specification/2025-06-18/server/tools) contracts.
The retained verifier exercises real stdio messages and the real CLI.

This companion trusts the configured executable, official bucket, manager state, and first-party Lua modules.
It is not an operating-system sandbox for a malicious backend or trusted module.
The backend can inspect registered paths during state validation.
Keep those paths inside the operator-approved roots.
Path checks are not atomic against concurrent filesystem changes.
The backend can allocate memory before its output reaches the companion's stream limit.
A forced write termination can leave a recoverable CLI transaction.
Use the normal CLI recovery flow outside MCP before another write.
No live game verification or cross-platform verification is implied by fixture results.

## Verification

Follow [.agents/skills/verify-modn-mcp/SKILL.md](../.agents/skills/verify-modn-mcp/SKILL.md).
The Python driver creates disposable game, archive, bucket, and manager fixtures.
It preserves protocol transcripts and a nonzero check count outside the fixtures.
It verifies default write refusal, preview byte preservation, explicit approval, core drift protection, progress notifications, and cancellation before and after the commit point.
