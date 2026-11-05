# MCP feature map

Build both binaries and the fault backend as described in [SKILL.md](../SKILL.md).
Run [verify.py](../verify.py) against disposable fixtures only.
The driver records JSON-RPC responses in `protocol.json` and named assertions in `summary.json`.

- [Protocol](protocol.md): initialization, IDs, errors, framing, and EOF.
- [Management](management.md): real CLI reads, previews, approvals, and ownership.
- [Boundaries](boundaries.md): scopes, backend errors, byte limits, and deadlines.

The driver covers these entry points through local stdio pipes.
Interactive third-party client interfaces and non-Windows platforms remain unverified.
