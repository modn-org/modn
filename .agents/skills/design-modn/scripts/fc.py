"""Call one FigCraft MCP tool on the app started with `figcraft --control PORT`.

Usage:
  fc.py [--port N] --list
  fc.py [--port N] TOOL [JSON_ARGS] [--png OUT.png]

Prints each text block of the result and saves the first image block to
--png. Exits 1 when the tool reports an error. The port defaults to
FIGCRAFT_CONTROL_PORT, else 47811.
"""

import argparse
import base64
import json
import os
import shutil
import subprocess
import sys

DEFAULT_CLI = r"C:\Program Files\ArtCraft\FigCraft\figcraft-cli.exe"


def find_cli():
    found = shutil.which("figcraft-cli")
    if found:
        return found
    if os.path.exists(DEFAULT_CLI):
        return DEFAULT_CLI
    sys.exit("figcraft-cli is not on PATH")


class Session:
    def __init__(self, port):
        self.port = port
        self.proc = subprocess.Popen(
            [find_cli(), "mcp", "--attach", str(port)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
            encoding="utf-8",
        )
        self.next_id = 1
        self.request("initialize", {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "design-modn", "version": "1"},
        })
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def send(self, message):
        self.proc.stdin.write(json.dumps(message) + "\n")
        self.proc.stdin.flush()

    def request(self, method, params):
        request_id = self.next_id
        self.next_id += 1
        self.send({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params})
        for line in self.proc.stdout:
            message = json.loads(line)
            if message.get("id") == request_id:
                if "error" in message:
                    sys.exit(f"{method}: {message['error']}")
                return message["result"]
        sys.exit(f"figcraft-cli stopped during {method}. Start the app with `figcraft --control {self.port}`.")

    def close(self):
        self.proc.stdin.close()
        self.proc.wait(timeout=30)


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=int(os.environ.get("FIGCRAFT_CONTROL_PORT", "47811")))
    parser.add_argument("--list", action="store_true")
    parser.add_argument("--png")
    parser.add_argument("tool", nargs="?")
    parser.add_argument("args", nargs="?", default="{}")
    options = parser.parse_args()
    if not options.list and not options.tool:
        parser.error("TOOL is required")

    session = Session(options.port)
    try:
        if options.list:
            for tool in session.request("tools/list", {})["tools"]:
                print(tool["name"])
            return 0
        result = session.request("tools/call", {"name": options.tool, "arguments": json.loads(options.args)})
    finally:
        session.close()

    saved = False
    for block in result.get("content", []):
        if block.get("type") == "text":
            print(block["text"])
        elif block.get("type") == "image" and options.png and not saved:
            with open(options.png, "wb") as out:
                out.write(base64.b64decode(block["data"]))
            print(f"saved {options.png}")
            saved = True
    return 1 if result.get("isError") else 0


if __name__ == "__main__":
    sys.exit(main())
