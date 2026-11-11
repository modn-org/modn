# Protocol

Clients initialize the companion and discover typed tools through stdio.

## Sub-features

P1 initialization; P2 ping and IDs; P3 typed discovery; P4 malformed messages; P5 EOF; P6 progress notifications; P7 cancellation.

## How to get to it (user POV)

Start `modn-mcp` with the fixed backend, manager path, and allowed roots.
Connect an MCP stdio client and send `initialize`, then `notifications/initialized`.

## Driving it with verify.py

Run the driver command in the parent skill.
Require five default tools, preserved string IDs, expected JSON-RPC error codes, and a clean EOF exit.
Require increasing progress notifications for a real install with a token, no response when a real install is cancelled before the commit point, unchanged fixtures after that cancel, and a normal response when a fault backend is cancelled after its commit point.
Read the named assertions and response transcript in the evidence directory.

## Gotchas

Messages require newline framing. Notifications do not receive responses.
MCP shutdown closes stdin; `shutdown` is an unknown method.
