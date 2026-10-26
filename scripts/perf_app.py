#!/usr/bin/env python3
"""App-level performance check (PLAN 9 row 6, AD26, P6-T3). Run through scripts/perf.sh --app.

Launches the release `berth` on the real engine with a throwaway data dir and measures:

  cold start  launch to the `projects_painted` timing mark (a populated Projects table on
              screen), after one priming launch that leaves a stored snapshot behind.
              Wall clock from just before exec, so dynamic linking is counted.
  memory      `footprint` (Activity Monitor's figure) of the app plus its WebKit helper
              processes, window open, with 50 containers on the engine.
  idle CPU    the window closed with Cmd-W, 60 s, deltas of `ps -o time=` (cumulative CPU
              time, not %cpu) summed over the app and its helpers.

WebKit's WebContent, GPU and Networking processes are not children of the app, so they are
found by start time: processes with a com.apple.WebKit name that did not exist before the
launch and started after it.

Needs Accessibility for osascript (Cmd-W, Cmd-Q) and the berthfx and berthfx-perf fixtures up
(50 containers) with berthfx-broken down. It touches no container.
"""

import argparse
import atexit
import json
import os
import re
import shutil
import signal
import sqlite3
import statistics
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
APP = os.path.join(ROOT, "target", "release", "berth")
TARGET_COLD_MS = 600
TARGET_IDLE_CPU_PCT = 0.5
TARGET_MEMORY_MB = 150
MARK = re.compile(r"timing name=(\w+) ms=(\d+) epoch_ms=(\d+)")


def out(*args):
    return subprocess.run(args, capture_output=True, text=True, check=False).stdout


def fixture_counts():
    names = out("docker", "ps", "-a", "--filter", "label=dev.berth.fixture=1", "--format", "{{.Names}}")
    names = [n for n in names.split() if n]
    return {
        "total": len(names),
        "perf": sum(n.startswith("berthfx-perf-") for n in names),
        "broken": sum(n.startswith("berthfx-broken-") for n in names),
    }


def check_fixtures():
    counts = fixture_counts()
    running = out("docker", "ps", "-q", "--filter", "label=dev.berth.fixture=1").split()
    if counts["broken"]:
        sys.exit("perf: berthfx-broken is up; take it down first (scripts/fx-down.sh would also remove berthfx)")
    if counts["perf"] < 46 or len(running) < 50:
        sys.exit(
            f"perf: need 50 running fixture containers (berthfx 4 + berthfx-perf 46), found {len(running)}: "
            "scripts/fx-up.sh and scripts/fx-up.sh --perf"
        )
    return len(running)


def cpu_seconds(pid):
    """Cumulative CPU time of a process from `ps -o time=` ([dd-][hh:]mm:ss.ss), or None."""
    text = out("ps", "-o", "time=", "-p", str(pid)).strip()
    if not text:
        return None
    days = 0
    if "-" in text:
        d, text = text.split("-", 1)
        days = int(d)
    seconds = 0.0
    for part in text.split(":"):
        seconds = seconds * 60 + float(part)
    return days * 86400 + seconds


def processes():
    """{pid: (lstart text, command)} for every process."""
    found = {}
    for line in out("ps", "-axo", "pid=,lstart=,comm=").splitlines():
        m = re.match(r"\s*(\d+)\s+(\w{3} \w{3}\s+\d+ [\d:]+ \d{4})\s+(.*)", line)
        if m:
            found[int(m.group(1))] = (m.group(2), m.group(3))
    return found


def webkit_pids():
    return {p for p, (_, comm) in processes().items() if "com.apple.WebKit" in comm}


def parse_lstart(text):
    return time.mktime(time.strptime(text, "%a %b %d %H:%M:%S %Y"))


class Run:
    """One launch of the app."""

    def __init__(self, data_dir, tag):
        self.log_path = os.path.join(data_dir, f"{tag}.log")
        self.before = webkit_pids()
        env = dict(os.environ, BERTH_DATA_DIR=data_dir, NO_COLOR="1")
        for var in ("BERTH_MOCK", "BERTH_SCENE", "BERTH_SEARCH_PATH"):
            env.pop(var, None)
        self.log = open(self.log_path, "w")
        self.launched_ms = int(time.time() * 1000)
        self.proc = subprocess.Popen([APP], env=env, stdout=subprocess.DEVNULL, stderr=self.log)
        self.pid = self.proc.pid
        atexit.register(self._kill)

    def _kill(self):
        """Leave no app behind when the script dies half way."""
        if self.proc.poll() is None:
            self.proc.kill()

    def marks(self):
        found = {}
        try:
            text = open(self.log_path, errors="replace").read()
        except OSError:
            return found
        for name, ms, epoch in MARK.findall(text):
            found.setdefault(name, (int(ms), int(epoch)))
        return found

    def wait_mark(self, name, timeout=60.0):
        end = time.time() + timeout
        while time.time() < end:
            marks = self.marks()
            if name in marks:
                return marks[name]
            if self.proc.poll() is not None:
                sys.exit(f"perf: the app exited before mark {name}; see {self.log_path}")
            time.sleep(0.05)
        sys.exit(f"perf: mark {name} not seen in {timeout:.0f} s; see {self.log_path}")

    def helpers(self):
        """The WebKit processes this launch started."""
        started = parse_lstart(processes()[self.pid][0]) - 1 if self.pid in processes() else 0
        mine = []
        for pid, (lstart, comm) in processes().items():
            if "com.apple.WebKit" in comm and pid not in self.before and parse_lstart(lstart) >= started:
                mine.append((pid, comm.rsplit("/", 1)[-1].replace("com.apple.WebKit.", "")))
        return sorted(mine)

    def activate(self):
        """Bring the app to the front once its process is known to System Events."""
        script = (
            'tell application "System Events"\n'
            f"  set frontmost of (first process whose unix id is {self.pid}) to true\n"
            "end tell"
        )
        for _ in range(20):
            if subprocess.run(["osascript", "-e", script], capture_output=True).returncode == 0:
                return
            time.sleep(0.05)

    def key(self, letter):
        script = (
            'tell application "System Events"\n'
            f"  set p to first process whose unix id is {self.pid}\n"
            "  set frontmost of p to true\n"
            "  delay 0.4\n"
            f'  keystroke "{letter}" using command down\n'
            "end tell"
        )
        result = subprocess.run(["osascript", "-e", script], capture_output=True, text=True)
        if result.returncode != 0:
            sys.exit(f"perf: osascript failed (Accessibility?): {result.stderr.strip()}")

    def on_screen(self):
        helper = os.path.join(ROOT, "target", "winbounds")
        source = os.path.join(ROOT, "scripts", "winbounds.swift")
        if not os.path.exists(helper) or os.path.getmtime(source) > os.path.getmtime(helper):
            subprocess.run(["swiftc", "-O", "-o", helper, source], check=True)
        return bool(out(helper, str(self.pid)).strip())

    def stop(self, graceful):
        if self.proc.poll() is None:
            if graceful:
                self.key("q")
                try:
                    self.proc.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    pass
            if self.proc.poll() is None:
                self.proc.send_signal(signal.SIGTERM)
                try:
                    self.proc.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    self.proc.kill()
                    self.proc.wait()
        self.log.close()
        # The helpers go with the app; give launchd a moment before the next launch counts them.
        time.sleep(1.5)


def footprints(pids):
    """{pid: bytes} from `footprint`."""
    path = tempfile.mktemp(suffix=".json")
    args = ["footprint", "-j", path]
    for pid in pids:
        args += ["-p", str(pid)]
    subprocess.run(args, capture_output=True, check=False)
    try:
        data = json.load(open(path))
    except (OSError, ValueError):
        return {}
    finally:
        if os.path.exists(path):
            os.unlink(path)
    unit = data.get("bytes per unit", 1)
    return {p["pid"]: p["footprint"] * unit for p in data.get("processes", [])}


def stored_snapshots(data_dir):
    try:
        db = sqlite3.connect(f"file:{os.path.join(data_dir, 'berth.db')}?mode=ro", uri=True)
        rows = db.execute("select count(*) from snapshots").fetchone()[0]
        db.close()
        return rows
    except sqlite3.Error:
        return 0


def cold_start(data_dir, runs):
    print("-- cold start: priming launch (leaves a stored snapshot), then", runs, "measured launches")
    prime = Run(data_dir, "prime")
    prime.activate()
    prime.wait_mark("projects_live")
    # The snapshot saver writes every 30 s; a Cmd-Q save is not reliable (the signals are
    # gone by then), so wait for the row itself.
    end = time.time() + 45
    while not stored_snapshots(data_dir) and time.time() < end:
        time.sleep(1)
    prime.stop(graceful=False)
    if not stored_snapshots(data_dir):
        sys.exit("perf: the priming launch stored no snapshot; see " + prime.log_path)
    results = []
    last = {}
    attempts = 0
    while len(results) < runs and attempts < runs + 4:
        attempts += 1
        run = Run(data_dir, f"cold{attempts}")
        run.activate()
        painted = run.wait_mark("projects_painted")
        run.wait_mark("projects_live")
        marks = run.marks()
        run.stop(graceful=False)
        last = {k: v[1] - run.launched_ms for k, v in marks.items()}
        # Animation frames stop while another window covers this one; the paint mark then
        # arrives late and says nothing about start-up.
        if painted[1] - marks["projects_snapshot"][1] > 250:
            print(f"   attempt {attempts}: window was covered, paint mark late; retrying")
            continue
        results.append(painted[1] - run.launched_ms)
        print(
            "   run %d: painted +%d ms | " % (len(results), results[-1])
            + " ".join(f"{k} +{v}" for k, v in sorted(last.items(), key=lambda kv: kv[1]))
        )
    if len(results) < runs:
        sys.exit("perf: the window stayed covered; close what is in front of it and run again")
    return statistics.median(results), results, last


def memory_and_idle(data_dir, settle, idle_secs):
    print("-- memory (window open) and idle CPU (window closed %d s)" % idle_secs)
    run = Run(data_dir, "idle")
    run.wait_mark("projects_live")
    time.sleep(settle)
    helpers = run.helpers()
    if not helpers:
        print("   warning: no WebKit helper processes found by start time")
    pids = [run.pid] + [p for p, _ in helpers]
    names = {run.pid: "app", **dict(helpers)}
    sizes = footprints(pids)
    total_mb = sum(sizes.values()) / 1048576
    parts = ", ".join(f"{names[p]} {sizes.get(p, 0) / 1048576:.0f}" for p in pids)
    print(f"   footprint {total_mb:.1f} MB ({parts})")

    run.key("w")
    time.sleep(3)
    if run.on_screen():
        print("   warning: the window is still on screen after Cmd-W")
    helpers = run.helpers()  # a helper may have started or gone since
    seen = {run.pid: names[run.pid], **dict(helpers)}
    first, last = {}, {}
    end = time.time() + idle_secs
    start = time.time()
    while True:
        for pid in seen:
            seconds = cpu_seconds(pid)
            if seconds is not None:
                first.setdefault(pid, seconds)
                last[pid] = seconds
        if time.time() >= end:
            break
        time.sleep(5)
        for pid, name in run.helpers():
            seen.setdefault(pid, name)
    elapsed = time.time() - start
    per = {pid: last.get(pid, 0) - first.get(pid, 0) for pid in seen}
    pct = sum(per.values()) / elapsed * 100
    detail = ", ".join(f"{seen[p]} {per[p] / elapsed * 100:.2f}" for p in seen)
    print(f"   cpu {pct:.2f} % over {elapsed:.0f} s ({detail})")
    run.stop(graceful=True)
    return total_mb, pct, parts


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--idle-secs", type=int, default=60)
    ap.add_argument("--settle", type=int, default=25, help="seconds after live data before memory is read")
    ap.add_argument("--no-build", action="store_true")
    args = ap.parse_args()

    containers = check_fixtures()
    if not args.no_build:
        subprocess.run(["cargo", "build", "--release", "-p", "berth-app"], cwd=ROOT, check=True)
    if not os.path.exists(APP):
        sys.exit(f"perf: {APP} not built")

    data_dir = tempfile.mkdtemp(prefix="berth-perf-")
    try:
        cold, runs, marks = cold_start(data_dir, args.runs)
        mem, cpu, parts = memory_and_idle(data_dir, args.settle, args.idle_secs)
    finally:
        shutil.rmtree(data_dir, ignore_errors=True)

    rows = [
        ("cold start", f"{cold:.0f} ms (runs: {' '.join(str(r) for r in runs)})", cold < TARGET_COLD_MS, f"< {TARGET_COLD_MS} ms"),
        ("idle CPU", f"{cpu:.2f} % (window closed)", cpu < TARGET_IDLE_CPU_PCT, f"< {TARGET_IDLE_CPU_PCT} %"),
        ("memory", f"{mem:.0f} MB (window open, {containers} containers; {parts})", mem < TARGET_MEMORY_MB, f"< {TARGET_MEMORY_MB} MB"),
    ]
    print()
    print("== App performance (release) ==")
    failed = 0
    for name, value, ok, target in rows:
        print(f"{name:<11} {value:<70} target {target:<9} {'ok' if ok else 'FAIL'}")
        failed += not ok
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
