# Management

Users inspect local packages and approve managed writes separately.

## Sub-features

M1 search and registrations; M2 archive inspection; M3 previews; M4 approval; M5 drift protection.

## How to get to it (user POV)

Call the five default tools through any MCP stdio client.
Start a separate server with `--allow-writes` to discover the two managed write tools.

## Driving it with verify.py

The driver uses real Modn CLI registration, search, inspection, preview, install, and uninstall.
Require unchanged file hashes after both previews.
Require explicit approval for writes and refusal to uninstall a changed owned file.
Require an approved install to preserve existing unmanaged content and refuse its occupied destination.
Require the unrelated game marker to survive removal.

## Gotchas

The synthetic executable identifies a disposable game. It cannot run the game.
The CLI's normal ownership and permission checks remain authoritative.
