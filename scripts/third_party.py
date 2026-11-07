#!/usr/bin/env python3
"""Third-party notices for the About dialog.

Lists every crate the app links (its normal dependencies with all
features) with its version, license and repository, from `cargo
metadata`. Writes assets/third-party.txt.
"""
import json
import subprocess
import sys

meta = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--format-version", "1", "--all-features", "--locked"]))
packages = {p["id"]: p for p in meta["packages"]}
members = set(meta["workspace_members"])
nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
root = next(i for i in members if packages[i]["name"] == "oxrok-ui")

seen, todo = set(), [root]
while todo:
    pid = todo.pop()
    for dep in nodes[pid]["deps"]:
        # Normal (linked) dependencies only: no build or dev ones.
        if any(k["kind"] is None for k in dep["dep_kinds"]) and dep["pkg"] not in seen:
            seen.add(dep["pkg"])
            todo.append(dep["pkg"])

rows = sorted((packages[i] for i in seen - members), key=lambda p: (p["name"], p["version"]))
out = ["Third-party software in this build", "",
       "The app links these Rust crates. Each is used under its license;",
       "the full texts ship with the crates' sources.", ""]
for p in rows:
    out.append(f"{p['name']} {p['version']}")
    out.append(f"    License: {p.get('license') or 'see repository'}")
    if p.get("repository"):
        out.append(f"    {p['repository']}")
out += ["", "The protocol decoders run on the RustPython interpreter built into",
        "the binary. They are Python source files from libsigrokdecode and keep",
        "their own licenses; each decoder's __init__.py names one."]
path = "assets/third-party.txt"
with open(path, "w") as f:
    f.write("\n".join(out) + "\n")
print(f"{path}: {len(rows)} crates", file=sys.stderr)
