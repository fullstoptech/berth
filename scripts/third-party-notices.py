#!/usr/bin/env python3
"""Write THIRD_PARTY_NOTICES.md (P10-T1, PLAN §12).

Reproducible, stdlib only. Run from anywhere:

    python3 -I scripts/third-party-notices.py > THIRD_PARTY_NOTICES.md

Needs `cargo` on PATH and, for the web bundles, web-src/node_modules (run
scripts/build-web.sh once). Three sources:

* the Rust crates that ship in the app: `cargo metadata` for aarch64-apple-darwin,
  normal (non-dev, non-build) dependencies reachable from berth-app and berth-coolify,
  grouped by licence expression;
* the npm packages bundled into assets/web/{terminal,editor}.js: the packages in
  web-src/pnpm-lock.yaml minus the build tools (esbuild, pnpm), with their licence and
  copyright line read from web-src/node_modules;
* the fonts embedded in the binary: the notice and licence text in
  crates/berth-app/assets/fonts/OFL.txt.

Exits non-zero when a crate has no licence expression or a web package is missing, so a
dependency change cannot slip a gap into the file.
"""
import json
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TARGET = "aarch64-apple-darwin"
SHIPPED_ROOTS = ("berth-app", "berth-coolify")
WEB_BUILD_TOOLS = ("esbuild", "@esbuild/", "@pnpm/", "pnpm")

MIT_TEXT = """Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE."""


def fail(message):
    print(f"third-party-notices: {message}", file=sys.stderr)
    sys.exit(1)


def normalize(expr):
    """`MIT/Apache-2.0` and `MIT / Apache-2.0` become `Apache-2.0 OR MIT`-style, sorted."""
    expr = expr.replace("/", " OR ")
    expr = re.sub(r"\s+", " ", expr).strip()
    if "(" in expr or " AND " in expr or " WITH " in expr:
        return expr
    parts = sorted({p.strip() for p in expr.split(" OR ") if p.strip()})
    return " OR ".join(parts)


def shipped_crates():
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", TARGET],
        cwd=ROOT, check=True, capture_output=True, text=True,
    ).stdout
    meta = json.loads(out)
    packages = {p["id"]: p for p in meta["packages"]}
    members = set(meta["workspace_members"])
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    roots = [i for i in members if packages[i]["name"] in SHIPPED_ROOTS]
    if len(roots) != len(SHIPPED_ROOTS):
        fail(f"expected workspace members {SHIPPED_ROOTS}, found {[packages[i]['name'] for i in roots]}")
    seen, stack = set(), list(roots)
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for dep in nodes[pid]["deps"]:
            if any(k["kind"] is None for k in dep["dep_kinds"]):
                stack.append(dep["pkg"])
    groups = defaultdict(list)
    for pid in seen:
        if pid in members:
            continue
        pkg = packages[pid]
        expr = pkg.get("license")
        if not expr:
            fail(f"{pkg['name']} {pkg['version']} declares no licence expression")
        groups[normalize(expr)].append((pkg["name"], pkg["version"]))
    for names in groups.values():
        names.sort()
    return groups


def web_packages():
    lock = (ROOT / "web-src/pnpm-lock.yaml").read_text()
    # The lockfile has two YAML documents: pnpm's own packages, then the project's.
    section = lock.split("\n---\n")[-1].split("\npackages:\n", 1)[1].split("\nsnapshots:\n", 1)[0]
    names = re.findall(r"^  '?((?:@[^/'@]+/)?[^@'\s]+)@([^':\s]+)'?:\s*$", section, re.M)
    rows = []
    for name, version in names:
        if name.startswith(WEB_BUILD_TOOLS):
            continue
        folder = ROOT / "web-src/node_modules/.pnpm" / (f"{name.replace('/', '+')}@{version}") / "node_modules" / name
        package_json = folder / "package.json"
        if not package_json.is_file():
            fail(f"{name}@{version} is not in web-src/node_modules; run scripts/build-web.sh")
        licence = json.loads(package_json.read_text()).get("license")
        if licence != "MIT":
            fail(f"{name}@{version} is {licence!r}; the MIT-only notice text needs a review")
        copyright_line = ""
        for candidate in sorted(folder.iterdir()):
            if candidate.name.upper().startswith("LICENSE"):
                for line in candidate.read_text(errors="ignore").splitlines():
                    if "copyright" in line.lower():
                        copyright_line = line.strip()
                        break
        if not copyright_line:
            fail(f"{name}@{version} has no copyright line in its LICENSE file")
        rows.append((name, version, copyright_line))
    rows.sort()
    return rows


def main():
    crates = shipped_crates()
    web = web_packages()
    ofl = (ROOT / "crates/berth-app/assets/fonts/OFL.txt").read_text().rstrip()
    total = sum(len(v) for v in crates.values())

    out = []
    w = out.append
    w("# Third-party notices\n")
    w("Berth is released under `MIT OR Apache-2.0` (`LICENSE-MIT`, `LICENSE-APACHE`). "
      "It ships with the software below, each under its own licence.\n")
    w("This file is generated: `python3 -I scripts/third-party-notices.py > THIRD_PARTY_NOTICES.md`. "
      "Do not edit it by hand.\n")

    w("## Fonts\n")
    w("Embedded in the application binary (`crates/berth-app/assets/fonts/`), under the SIL Open Font "
      "License 1.1. The notice and licence below are the file `OFL.txt` shipped with the fonts.\n")
    w("| Font | Files |\n|---|---|")
    w("| IBM Plex Sans | `IBMPlexSans-400.ttf`, `-500.ttf`, `-600.ttf` |")
    w("| JetBrains Mono | `JetBrainsMono-400.ttf`, `-500.ttf` |\n")
    w("```text\n" + ofl + "\n```\n")

    w("## Terminal and editor bundles (npm)\n")
    w("`assets/web/terminal.js` and `assets/web/editor.js` are built by `scripts/build-web.sh` from "
      "`web-src/` (versions pinned in `web-src/pnpm-lock.yaml`). Every package below is MIT licensed "
      "under the copyright line shown; the licence text follows the table.\n")
    w("| Package | Version | Copyright |\n|---|---|---|")
    for name, version, line in web:
        w(f"| {name} | {version} | {line} |")
    w("\n```text\n" + MIT_TEXT + "\n```\n")

    w("## Rust crates\n")
    w(f"{total} crates are compiled into `berth` and `berth-coolify` for `{TARGET}` (normal dependencies "
      "of both, from `cargo metadata`; build-only, dev-only and other-platform crates are left out). "
      "Grouped by licence expression. Where an expression offers a choice (`OR`), Berth takes the "
      "terms of any one alternative. Each crate's full licence text and copyright notices are in its "
      "published source package on crates.io.\n")
    for expr in sorted(crates, key=lambda e: (-len(crates[e]), e)):
        items = crates[expr]
        w(f"### {expr} ({len(items)})\n")
        w(", ".join(f"{n} {v}" for n, v in items) + "\n")
        if "MPL-2.0" in expr:
            w("These are used unmodified; their source is the crates.io package of the same name and "
              "version (MPL-2.0 file-level terms).\n")

    sys.stdout.write("\n".join(out).rstrip("\n") + "\n")


if __name__ == "__main__":
    main()
