# Boundaries

The companion bounds its local inputs and backend responses.

## Sub-features

B1 path scopes; B2 option injection; B3 oversized framing; B4 output limits; B5 timeouts and EOF.

## How to get to it (user POV)

Submit a disallowed path or malformed tool input through the MCP client.
Backend failures appear as MCP tool errors.

## Driving it with verify.py

Run the driver with the separately compiled fault backend.
Require deterministic errors for outside paths, traversal, unknown fields, disabled writes, and invalid IDs.
Require stream-limit failures for large stdout and stderr, plus timeout and nonzero-exit errors.
Require refusal when a junction or symlink replaces the manager root after startup.
Require the redirected target to remain empty.
Require the server to exit within four seconds after EOF with its one-second test backend deadline.

## Gotchas

The fault backend is only a verification helper. The real CLI journeys use `modn`.
The companion does not isolate a malicious executable or concurrent filesystem attacker.
