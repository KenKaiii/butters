"""Real-app end-to-end test for macOS: drives the actual built Butters.app with
real mouse clicks, and checks the results on disk and on screen.

    pnpm tauri build --debug --bundles app --no-sign   # once
    python3 e2e/macos_app.py [path/to/debug/Butters.app]

It runs a *debug* build of Butters with a throwaway HOME, so your real settings
are untouched, and a throwaway dev server (a leftover `node` with an open port)
for it to find and close. BUTTERS_E2E_ONLY_PID (debug builds only) limits the
suggestions to that one process, so the test can never close your real work.
Any running Butters is quit first and reopened afterwards. Needs: macOS, Python 3 with pyobjc (Quartz), and Accessibility
permission for the terminal running it (System Settings > Privacy & Security >
Accessibility), since it clicks like a person would.

Checks:
  1. The panel opens from a second launch (single instance).
  2. Clicking the Sounds switch saves sounds=false to settings.json.
  3. Clicking Alerts saves alerts=false; clicking again saves it back.
  4. A leftover dev server is found, closed with Close -> Close, and its
     process is really gone.
  5. Clicking outside closes the panel and the window is destroyed.
  6. The global shortcut (⌥⌘B) opens and closes the panel.
"""
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

import Quartz

ROOT = Path(__file__).resolve().parent.parent
APP = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "src-tauri/target/debug/bundle/macos/Butters.app"
BIN = APP / "Contents/MacOS/butters"
IDENT = json.load(open(ROOT / "src-tauri/tauri.conf.json"))["identifier"]

# Panel layout in points, relative to the window's top-left (see panel.rs MARGIN
# and App.svelte). The window has a 12 pt side margin and 4 pt top margin.
SETTINGS_BTN = (361, 35)
SWITCH_X = 359
ALERTS_Y, SOUNDS_Y = 94, 157


def click(x, y):
    for kind in (Quartz.kCGEventMouseMoved, Quartz.kCGEventLeftMouseDown, Quartz.kCGEventLeftMouseUp):
        Quartz.CGEventPost(Quartz.kCGHIDEventTap, Quartz.CGEventCreateMouseEvent(None, kind, (x, y), Quartz.kCGMouseButtonLeft))
        time.sleep(0.05)


def shortcut(flags, keycode):
    for down in (True, False):
        e = Quartz.CGEventCreateKeyboardEvent(None, keycode, down)
        Quartz.CGEventSetFlags(e, flags)
        Quartz.CGEventPost(Quartz.kCGHIDEventTap, e)
        time.sleep(0.05)


def windows_of(pid):
    wins = Quartz.CGWindowListCopyWindowInfo(Quartz.kCGWindowListOptionOnScreenOnly, Quartz.kCGNullWindowID)
    return [w for w in wins if w.get("kCGWindowOwnerPID") == pid and w.get("kCGWindowLayer", 0) >= 0 and w["kCGWindowBounds"]["Width"] > 300]


def wait(pred, timeout=8.0, what="condition"):
    end = time.time() + timeout
    while time.time() < end:
        v = pred()
        if v:
            return v
        time.sleep(0.15)
    raise AssertionError(f"timed out waiting for {what}")


class Run:
    def __init__(self):
        self.home = Path(tempfile.mkdtemp(prefix="butters-e2e-"))
        self.env = {**os.environ, "HOME": str(self.home)}
        self.settings = self.home / "Library/Application Support" / IDENT / "settings.json"
        self.procs = []

    def start(self, only_pid):
        self.env["BUTTERS_E2E_ONLY_PID"] = str(only_pid)
        p = subprocess.Popen([str(BIN)], env=self.env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.procs.append(p)
        self.pid = p.pid
        time.sleep(2.5)

    def open_panel(self):
        subprocess.run([str(BIN)], env=self.env, timeout=10, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        w = wait(lambda: windows_of(self.pid), what="panel window")[0]["kCGWindowBounds"]
        self.x, self.y = w["X"], w["Y"]
        # Let it pour in and the first scan land (first launch on a cold
        # machine is slower; this is the one place a fixed wait was flaky).
        time.sleep(1.6)

    def at(self, dx, dy):
        click(self.x + dx, self.y + dy)

    def saved(self):
        try:
            return json.loads(self.settings.read_text())
        except (FileNotFoundError, json.JSONDecodeError):
            return None

    def stop(self):
        for p in self.procs:
            if p.poll() is None:
                p.send_signal(signal.SIGTERM)
                try:
                    p.wait(5)
                except subprocess.TimeoutExpired:
                    p.kill()
        shutil.rmtree(self.home, ignore_errors=True)


def leftover_server():
    """A throwaway 'dev server' whose terminal is gone: node listening on a port, parent = launchd."""
    script = "require('http').createServer((q,s)=>s.end('ok')).listen(0,'127.0.0.1');setInterval(()=>{},1e9)"
    # Double-fork via sh so the node process is re-parented to launchd (orphaned).
    out = subprocess.run(["/bin/sh", "-c", f"nohup node -e \"{script}\" >/dev/null 2>&1 & echo $!"], capture_output=True, text=True)
    return int(out.stdout.strip())


def alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False


def main():
    assert BIN.exists(), f"build the debug app first: {BIN}"
    was_running = subprocess.run(["pgrep", "-f", "Butters.app/Contents/MacOS/butters"], capture_output=True).returncode == 0
    subprocess.run(["pkill", "-f", "Butters.app/Contents/MacOS/butters"])
    time.sleep(1)
    results = []
    run = Run()
    server = leftover_server()
    try:
        assert alive(server), "couldn't start the throwaway dev server"
        run.start(only_pid=server)
        run.open_panel()
        results.append(("panel opens from a second launch", True))

        # Open Settings, retrying once: the first launch after a fresh build is
        # slow while macOS checks the new app, and a click can land too early.
        for attempt in range(2):
            run.at(*SETTINGS_BTN)
            time.sleep(1.2)
            before = run.saved()
            run.at(SWITCH_X, SOUNDS_Y)
            try:
                wait(lambda: (v := run.saved()) and v.get("sounds") is False, timeout=4, what="sounds=false")
                break
            except AssertionError:
                if attempt == 1:
                    raise AssertionError("timed out waiting for Sounds switch to save sounds=false")
                # Not on Settings yet (or the click toggled nothing): go back to a known state.
                click(run.x - 200, run.y + 300)
                wait(lambda: not windows_of(run.pid), what="panel to close for retry")
                run.open_panel()
        if os.environ.get("E2E_SHOT"):
            subprocess.run(["screencapture", "-x", "-R", f"{run.x},{run.y},404,604", "/tmp/e2e-settings.png"])
        results.append(("Sounds switch saves", True))

        run.at(SWITCH_X, ALERTS_Y)
        wait(lambda: (v := run.saved()) and v.get("alerts") is False, what="Alerts switch to save alerts=false")
        run.at(SWITCH_X, ALERTS_Y)
        wait(lambda: (v := run.saved()) and v.get("alerts") is True, what="Alerts switch to save alerts=true")
        results.append(("Alerts switch saves both ways", True))

        click(run.x - 200, run.y + 300)  # click outside: panel melts away
        wait(lambda: not windows_of(run.pid), what="panel to close when clicking away")
        results.append(("clicking away closes and destroys the panel", True))

        # Global shortcut (⌥⌘B by default) opens the panel from anywhere, and closes it.
        shortcut(Quartz.kCGEventFlagMaskAlternate | Quartz.kCGEventFlagMaskCommand, 11)  # keycode 11 = B
        wait(lambda: windows_of(run.pid), what="⌥⌘B to open the panel")
        time.sleep(0.8)
        shortcut(Quartz.kCGEventFlagMaskAlternate | Quartz.kCGEventFlagMaskCommand, 11)
        wait(lambda: not windows_of(run.pid), what="⌥⌘B again to close it")
        results.append(("global shortcut opens and closes the panel", True))

        # Only the throwaway server can be suggested; it's a leftover, so pre-ticked.
        run.open_panel()
        time.sleep(1.0)
        run.at(330, 580 - 30)  # "Close 1"
        time.sleep(0.6)
        run.at(345, 580 - 30)  # confirm "Close"
        wait(lambda: not alive(server), timeout=12, what="the leftover dev server to exit")
        results.append(("leftover dev server closed for real", True))
    except AssertionError as e:
        results.append((str(e), False))
    finally:
        if alive(server):
            os.kill(server, signal.SIGKILL)
        run.stop()
        if was_running and Path("/Applications/Butters.app").exists():
            subprocess.run(["open", "/Applications/Butters.app"])

    width = max(len(n) for n, _ in results)
    for name, ok in results:
        print(f"{'PASS' if ok else 'FAIL'}  {name.ljust(width)}")
    sys.exit(0 if all(ok for _, ok in results) else 1)


if __name__ == "__main__":
    main()
