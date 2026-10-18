import argparse
import importlib.util
import json
import re
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import time


spec = importlib.util.spec_from_file_location(
    "productivity_smoke", Path(__file__).with_name("test-productivity.py"))
productivity = importlib.util.module_from_spec(spec)
spec.loader.exec_module(productivity)
smoke = productivity.smoke


class Fixture(productivity.Fixture):
    def write_config(self, selected_language):
        super().write_config(selected_language)
        self.config.write_text(self.config.read_text().replace(
            "confirm_tab_close = false", "confirm_tab_close = true"))
        self.cell_size = None
        self.probe_sequence = 0

    def start(self):
        super().start()
        if self.cell_size is None:
            _, _, width, height = super().dimensions()
            self.cell_size = width, height
        self.command("PS1=''; PS2='' ")

    def dimensions(self):
        if self.cell_size is None:
            return super().dimensions()
        width, height = self.x11.geometry(self.window)
        cell_width, cell_height = self.cell_size
        return round(width / cell_width), round(height / cell_height) - 1, cell_width, cell_height

    def probe(self, label):
        self.probe_sequence += 1
        path = self.work / f"probe-{self.probe_sequence}-{label}"
        self.command("{ printf '%s\\n%s\\n' \"$$\" \"$PWD\"; stty size; } > "
            + shlex.quote(str(path)))
        smoke.wait_for(lambda: path.exists() and len(path.read_text().splitlines()) == 3,
            label + " shell and PTY geometry reported")
        pid, cwd, size = path.read_text().splitlines()
        rows, columns = map(int, size.split())
        return {"pid": int(pid), "cwd": cwd, "rows": rows, "columns": columns}

    def initialize(self, directory, label):
        self.command("PS1=''; PS2=''; cd " + shlex.quote(str(directory)))
        return self.probe(label)

    def split(self, key):
        self.x11.chord("Control_L", "Shift_L", key)
        time.sleep(0.25)

    def focus(self, direction):
        self.x11.chord("Control_L", "Alt_L", direction)

    def focus_rect(self, rect):
        x, y, width, height = rect
        self.click(x + width // 2, y + height // 2)

    def move(self, column, line):
        _, _, cell_width, cell_height = self.dimensions()
        self.x11.xtest.XTestFakeMotionEvent(self.x11.display, -1,
            int((column + 0.5) * cell_width), int((line + 1.5) * cell_height), 0)
        self.x11.lib.XFlush(self.x11.display)
        time.sleep(0.08)

    def button(self, pressed):
        self.x11.xtest.XTestFakeButtonEvent(self.x11.display, 1, pressed, 0)
        self.x11.lib.XFlush(self.x11.display)
        time.sleep(0.08)

    def drag(self, start, end):
        _, _, cell_width, cell_height = self.dimensions()
        point = lambda p: (int((p[0] + 0.5) * cell_width), int((p[1] + 1.5) * cell_height))
        x, y = point(start)
        target_x, target_y = point(end)
        test = self.x11.xtest
        test.XTestFakeMotionEvent(self.x11.display, -1, x, y, 0)
        test.XTestFakeButtonEvent(self.x11.display, 1, True, 0)
        self.x11.lib.XFlush(self.x11.display)
        time.sleep(0.1)
        for step in range(1, 6):
            test.XTestFakeMotionEvent(self.x11.display, -1,
                x + (target_x - x) * step // 5, y + (target_y - y) * step // 5, 0)
            self.x11.lib.XFlush(self.x11.display)
            time.sleep(0.04)
        test.XTestFakeButtonEvent(self.x11.display, 1, False, 0)
        self.x11.lib.XFlush(self.x11.display)
        time.sleep(0.2)

    def paint(self, label, color, flood=False):
        ready = self.work / ("painted-" + label)
        ready.unlink(missing_ok=True)
        self.command("python3 " + " ".join(shlex.quote(str(value)) for value in (
            self.work / "paint.py", label, color, int(flood), ready)))
        smoke.wait_for(ready.exists, label + " output painted inside its pane")
        time.sleep(0.15)

    def crop(self, image, rect):
        x, y, width, height = rect
        _, _, cell_width, cell_height = self.dimensions()
        geometry = f"{int(width * cell_width)}x{int(height * cell_height)}+{int(x * cell_width)}+{int((y + 1) * cell_height)}"
        return subprocess.run(["convert", str(image), "-crop", geometry, "+repage", "-depth", "8",
            "rgb:-"], capture_output=True, check=True, timeout=5).stdout

    def resize(self, columns, lines):
        _, _, cell_width, cell_height = self.dimensions()
        width, height = round(columns * cell_width), round((lines + 1) * cell_height)
        self.x11.lib.XResizeWindow(self.x11.display, self.window, width, height)
        self.x11.lib.XFlush(self.x11.display)
        smoke.wait_for(lambda: self.x11.geometry(self.window) == (width, height),
            f"window resized to {columns} columns and {lines} terminal rows")
        time.sleep(0.15)

    def close_pane(self, answer):
        self.x11.chord("Control_L", "Alt_L", "w")
        self.x11.chord(answer)

    def stop(self):
        if self.app is None or self.app.poll() is not None:
            return
        for _ in range(24):
            if self.app.poll() is not None:
                break
            self.x11.chord("Escape")
            self.x11.chord("Control_L", "Shift_L", "w")
            self.x11.chord("y")
        smoke.wait_for(lambda: self.app.poll() == 0, "all pane fixture tabs closed")
        self.app = None


def alive(pid):
    try:
        return Path(f"/proc/{pid}/stat").read_text().split(")", 1)[1].split()[0] != "Z"
    except FileNotFoundError:
        return False


def pane_rectangles(left, upper, lower):
    return {
        "a": (0, 0, left["columns"], left["rows"]),
        "b": (left["columns"] + 1, 0, upper["columns"], upper["rows"]),
        "c": (left["columns"] + 1, upper["rows"] + 1, lower["columns"], lower["rows"]),
    }


def saved_panes(tab):
    return [tab, *tab.get("panes", [])]


def check_initial_panes(fixture, directories):
    columns, rows, _, _ = fixture.dimensions()
    a = fixture.initialize(directories["a"], "first")
    fixture.rename("Daily panes")
    fixture.split("e")
    inherited_b = fixture.probe("right-inherited")
    assert inherited_b["cwd"] == a["cwd"]
    assert inherited_b["pid"] != a["pid"]
    b = fixture.initialize(directories["b"], "right")
    fixture.focus("Left")
    left = fixture.probe("left-focus")
    assert left["pid"] == a["pid"]
    assert left["columns"] + b["columns"] + 1 == columns
    assert left["rows"] == b["rows"] == rows
    fixture.focus("Right")
    assert fixture.probe("right-focus")["pid"] == b["pid"]
    fixture.split("j")
    inherited_c = fixture.probe("bottom-inherited")
    assert inherited_c["cwd"] == b["cwd"]
    c = fixture.initialize(directories["c"], "bottom")
    assert len({a["pid"], b["pid"], c["pid"]}) == 3
    fixture.focus("Up")
    upper = fixture.probe("upper-focus")
    assert upper["pid"] == b["pid"]
    assert upper["rows"] + c["rows"] + 1 == rows
    assert upper["columns"] == c["columns"]
    fixture.focus("Down")
    assert fixture.probe("lower-focus")["pid"] == c["pid"]
    rectangles = pane_rectangles(left, upper, c)
    fixture.focus_rect(rectangles["a"])
    assert fixture.probe("mouse-left")["pid"] == a["pid"]
    fixture.focus_rect(rectangles["c"])
    assert fixture.probe("mouse-bottom")["pid"] == c["pid"]
    return {"a": left, "b": upper, "c": c}, rectangles


def check_protocol_routing(fixture, panes, rectangles):
    def start_probe(mode, label=None):
        ready, gate, result = (fixture.work / ((label or mode) + suffix)
            for suffix in ("-ready", "-gate", "-result"))
        fixture.command("python3 " + " ".join(shlex.quote(str(value)) for value in (
            fixture.work / "protocol.py", mode, ready, gate, result)))
        smoke.wait_for(ready.exists, mode + " raw PTY ready")
        return gate, result

    gate, result = start_probe("query")
    fixture.focus_rect(rectangles["a"])
    gate.write_text("query")
    smoke.wait_for(result.exists, "background pane received its terminal query reply")
    match = re.fullmatch(rb"\x1b\[(\d+);(\d+)R", result.read_bytes())
    assert match, ("Cursor report delivered to wrong pane", result.read_bytes())
    row, column = map(int, match.groups())
    assert 1 <= row <= panes["c"]["rows"] and 1 <= column <= panes["c"]["columns"]
    assert fixture.probe("query-focused-survivor")["pid"] == panes["a"]["pid"]
    fixture.focus_rect(rectangles["c"])
    gate, result = start_probe("mouse")
    gate.write_text("mouse")
    x, y, _, _ = rectangles["c"]
    fixture.click(x + 4, y + 3)
    smoke.wait_for(result.exists, "offset pane received application mouse events")
    assert b"\x1b[<0;5;4M" in result.read_bytes(), result.read_bytes()
    assert b"\x1b[<0;5;4m" in result.read_bytes(), result.read_bytes()
    assert fixture.probe("mouse-protocol-survivor")["pid"] == panes["c"]["pid"]
    gate, result = start_probe("mouse", "mouse-divider")
    gate.write_text("mouse")
    fixture.move(x + 4, y + 3)
    fixture.button(True)
    fixture.move(panes["a"]["columns"], y + 3)
    fixture.button(False)
    smoke.wait_for(result.exists, "terminal drag released over a pane divider")
    events = re.findall(rb"\x1b\[<(\d+);(\d+);(\d+)([Mm])", result.read_bytes())
    assert len(events) == 2 and events[0] == (b"0", b"5", b"4", b"M"), result.read_bytes()
    assert events[1][0] == b"0" and events[1][3] == b"m", result.read_bytes()
    assert fixture.probe("divider-release-survivor")["pid"] == panes["c"]["pid"]

def check_rendering(fixture, panes, rectangles):
    colors = {"a": "612020", "b": "205120", "c": "203a68"}
    for label, color in colors.items():
        fixture.focus_rect(rectangles[label])
        fixture.paint(label, color)
    before = fixture.capture("nested-panes")
    regions = {label: (rect[0] + 2, rect[1] + 3, min(12, rect[2] - 4), 3)
        for label, rect in rectangles.items()}
    for label, region in regions.items():
        pixels = fixture.crop(before, region)
        assert pixels.count(bytes.fromhex(colors[label])) > len(pixels) // 4, label
    fixture.paint("c-flood", colors["c"], flood=True)
    after = fixture.capture("clipped-output")
    for label in ("a", "b"):
        assert fixture.crop(before, regions[label]) == fixture.crop(after, regions[label]), \
            "Long output escaped its pane into " + label
    fixture.x11.chord("Control_L", "Shift_L", "f")
    fixture.x11.type("pane")
    search = fixture.capture("pane-search")
    x, y, width, height = rectangles["c"]
    footer = fixture.pixels(search, x + 1, y + height - 1, min(10, width - 2))
    assert footer.count(bytes.fromhex("dddddd")) > len(footer) // 6, "Search missing from focused pane"
    for label in ("a", "b"):
        assert fixture.crop(after, regions[label]) == fixture.crop(search, regions[label]), \
            "Search changed another pane: " + label
    fixture.x11.chord("Escape")
    assert fixture.probe("search-closed") == panes["c"]
    fixture.x11.chord("Control_L", "comma")
    settings = fixture.capture("unified-settings")
    columns, rows, _, _ = fixture.dimensions()
    assert fixture.crop(settings, (columns // 4, rows // 4, columns // 2, rows // 2)) != \
        fixture.crop(after, (columns // 4, rows // 4, columns // 2, rows // 2))
    fixture.x11.chord("Escape")
    assert fixture.probe("settings-cancelled")["pid"] == panes["c"]["pid"]


def check_resize_zoom(fixture, panes, rectangles):
    columns, rows, _, _ = fixture.dimensions()
    fixture.move(panes["a"]["columns"], 3)
    fixture.button(True)
    fixture.x11.chord("Control_L", "comma")
    fixture.button(False)
    fixture.x11.chord("Escape")
    fixture.move(panes["a"]["columns"] + 10, 3)
    assert fixture.probe("modal-cancelled-divider-drag") == panes["c"]
    fixture.x11.chord("Control_L", "Shift_L", "z")
    zoomed = fixture.probe("maximized")
    assert zoomed["pid"] == panes["c"]["pid"]
    assert (zoomed["columns"], zoomed["rows"]) == (columns, rows)
    fixture.capture("maximized-pane")
    fixture.x11.chord("Control_L", "Shift_L", "z")
    assert fixture.probe("unmaximized") == panes["c"]
    fixture.drag((panes["a"]["columns"], 3), (panes["a"]["columns"] + 8, 3))
    fixture.focus("Left")
    left = fixture.probe("drag-left")
    assert left["pid"] == panes["a"]["pid"]
    assert left["columns"] > panes["a"]["columns"]
    fixture.focus_rect((left["columns"] + 1, 0, columns - left["columns"] - 1, panes["b"]["rows"]))
    upper = fixture.probe("drag-upper")
    assert upper["pid"] == panes["b"]["pid"]
    assert upper["columns"] < panes["b"]["columns"]
    fixture.drag((columns - 5, upper["rows"]), (columns - 5, upper["rows"] + 3))
    upper = fixture.probe("drag-horizontal")
    assert upper["rows"] > panes["b"]["rows"]
    fixture.focus("Down")
    lower = fixture.probe("drag-lower")
    assert lower["pid"] == panes["c"]["pid"]
    assert upper["rows"] + lower["rows"] + 1 == rows
    assert left["columns"] + lower["columns"] + 1 == columns
    fixture.capture("resized-dividers")
    fixture.resize(45, 18)
    compact = fixture.probe("compact")
    assert compact["pid"] == lower["pid"]
    assert compact["columns"] >= 2 and compact["rows"] >= 1
    fixture.capture("compact-panes")
    fixture.resize(columns, rows)
    assert fixture.probe("resized-window") == lower
    return {"a": left, "b": upper, "c": lower}, pane_rectangles(left, upper, lower)


def check_persistence(fixture, panes, rectangles, directories):
    before = fixture.snapshot()
    assert len(before["tabs"]) == 1
    tab = before["tabs"][0]
    saved = saved_panes(tab)
    assert len(saved) == 3
    assert {pane["working_directory"] for pane in saved} == {str(path) for path in directories.values()}
    assert saved[tab["active_pane"]]["working_directory"] == str(directories["c"])
    assert all(not pane["replay_last_command"] for pane in saved)
    layout = tab["layout"]
    assert layout["split"]["axis"] == "vertical"
    assert layout["split"]["ratio"] != 500
    assert layout["split"]["second"]["split"]["axis"] == "horizontal"
    assert layout["split"]["second"]["split"]["ratio"] != 500
    fixture.save_workspace("Daily panes")
    workspaces = json.loads((fixture.state_directory / "workspaces.json").read_text())
    assert workspaces["workspaces"]["Daily panes"]["tabs"][0]["layout"] == layout
    fixture.general(restore_session=True, startup_session="Daily panes")
    fixture.stop()
    smoke.wait_for(lambda: all(not alive(pane["pid"]) for pane in panes.values()), "closed all saved pane shells")
    fixture.start()
    active = fixture.probe("restored-active")
    assert active["cwd"] == str(directories["c"])
    assert (active["rows"], active["columns"]) == (panes["c"]["rows"], panes["c"]["columns"])
    restored = {}
    for label, rect in rectangles.items():
        fixture.focus_rect(rect)
        pane = fixture.probe("restored-" + label)
        assert pane["cwd"] == str(directories[label])
        assert pane["pid"] not in {item["pid"] for item in panes.values()}
        assert (pane["rows"], pane["columns"]) == (panes[label]["rows"], panes[label]["columns"])
        restored[label] = pane
    assert len({pane["pid"] for pane in restored.values()}) == 3
    assert fixture.snapshot()["tabs"][0]["layout"] == layout
    fixture.capture("restored-layout")
    return restored


def check_pending_group_closure(fixture):
    fixture.start()
    survivor = fixture.probe("pending-group-survivor")

    def gate_exit(label):
        report = fixture.probe(label)
        ready, gate = (fixture.work / (label + suffix) for suffix in ("-ready", "-gate"))
        fixture.command(f"printf ready > {shlex.quote(str(ready))}; "
            f"while test ! -e {shlex.quote(str(gate))}; do sleep 0.03; done; exit")
        smoke.wait_for(ready.exists, label + " waits for natural exit")
        return report["pid"], gate

    def pending_group(label):
        fixture.x11.chord("Control_L", "Shift_L", "t")
        time.sleep(0.2)
        first_pid, first_gate = gate_exit(label + "-first")
        fixture.split("e")
        second_pid, second_gate = gate_exit(label + "-second")
        fixture.x11.chord("Control_L", "Shift_L", "w")
        first_gate.write_text("exit")
        smoke.wait_for(lambda: not alive(first_pid), label + " original pane exited during group prompt")
        assert alive(second_pid) and alive(survivor["pid"])
        return second_pid, second_gate

    second_pid, _ = pending_group("pending-confirm")
    fixture.capture("group-prompt-after-first-exit")
    fixture.x11.chord("y")
    smoke.wait_for(lambda: not alive(second_pid), "group confirmation still closed surviving sibling")
    assert fixture.probe("group-confirmed-survivor")["pid"] == survivor["pid"]
    second_pid, second_gate = pending_group("pending-natural")
    second_gate.write_text("exit")
    smoke.wait_for(lambda: not alive(second_pid), "final sibling exited under group prompt")
    assert fixture.app.poll() is None
    assert fixture.probe("group-natural-survivor")["pid"] == survivor["pid"]
    assert len(fixture.snapshot()["tabs"]) == 1
    fixture.stop()

def check_closing(fixture, panes, rectangles):
    for answer in ("n", "Escape"):
        fixture.close_pane(answer)
        assert fixture.probe("cancel-pane-" + answer)["pid"] == panes["c"]["pid"]
        assert all(alive(pane["pid"]) for pane in panes.values())
    fixture.close_pane("y")
    smoke.wait_for(lambda: not alive(panes["c"]["pid"]), "only the confirmed pane exited")
    assert alive(panes["a"]["pid"]) and alive(panes["b"]["pid"])
    fixture.focus_rect(rectangles["b"])
    remaining = fixture.probe("after-pane-close")
    assert remaining["pid"] == panes["b"]["pid"]
    assert remaining["rows"] == fixture.dimensions()[1]
    fixture.close_pane("Return")
    smoke.wait_for(lambda: not alive(panes["b"]["pid"]), "Enter confirmed the second pane closure")
    remaining = fixture.probe("single-survivor")
    assert remaining["pid"] == panes["a"]["pid"]
    assert (remaining["columns"], remaining["rows"]) == fixture.dimensions()[:2]
    fixture.x11.chord("Control_L", "Shift_L", "w")
    fixture.x11.chord("Escape")
    assert fixture.probe("cancel-last-tab")["pid"] == panes["a"]["pid"]
    fixture.split("e")
    extra = fixture.probe("whole-tab-second-pane")
    fixture.general(restore_session=False, startup_session="")
    fixture.x11.chord("Control_L", "Shift_L", "w")
    fixture.x11.chord("y")
    smoke.wait_for(lambda: fixture.app.poll() == 0, "whole tab closure ended all panes")
    smoke.wait_for(lambda: not alive(extra["pid"]) and not alive(panes["a"]["pid"]),
        "whole tab closure left no pane shells")
    fixture.app = None
    fixture.start()
    first = fixture.probe("natural-first")
    fixture.split("e")
    second = fixture.probe("natural-second")
    fixture.command("exit")
    smoke.wait_for(lambda: not alive(second["pid"]), "natural pane exit skipped confirmation")
    assert fixture.app.poll() is None
    assert fixture.probe("natural-survivor")["pid"] == first["pid"]
    fixture.command("exit")
    smoke.wait_for(lambda: fixture.app.poll() == 0, "natural final shell exit closed the window")
    fixture.app = None


PROTOCOL_PROBE = """import os, pathlib, select, sys, termios, time, tty
mode, ready, gate, result = sys.argv[1:]
original = termios.tcgetattr(0)
captured = bytearray()
try:
    tty.setraw(0)
    if mode == 'mouse': os.write(1, b'\\x1b[?1000h\\x1b[?1006h')
    pathlib.Path(ready).write_text('ready')
    while not pathlib.Path(gate).exists(): time.sleep(0.02)
    if mode == 'query': os.write(1, b'\\x1b[6n')
    deadline = time.monotonic() + 3
    while time.monotonic() < deadline:
        if select.select([0], [], [], 0.1)[0]:
            captured.extend(os.read(0, 1024))
            if mode == 'query' and captured.endswith(b'R'): break
            if mode == 'mouse' and captured.endswith(b'm'): break
finally:
    if mode == 'mouse': os.write(1, b'\\x1b[?1000l\\x1b[?1006l')
    termios.tcsetattr(0, termios.TCSADRAIN, original)
pathlib.Path(result).write_bytes(captured)
"""

def run(binary, screenshot):
    for executable in ("Xvfb", "import", "convert", binary):
        if not shutil.which(executable):
            raise RuntimeError(f"Missing executable: {executable}")
    with tempfile.TemporaryDirectory(prefix="alacritty-panes-") as temporary:
        fixture = Fixture(binary, Path(temporary), screenshot)
        try:
            (fixture.work / "protocol.py").write_text(PROTOCOL_PROBE)
            (fixture.work / "paint.py").write_text(
                "import os, pathlib, sys\n"
                "label, color, flood, ready = sys.argv[1:]\n"
                "red, green, blue = (int(color[i:i+2], 16) for i in (0, 2, 4))\n"
                "data = f'\\x1b[?25l\\x1b[48;2;{red};{green};{blue}m\\x1b[2J\\x1b[H{label}'\n"
                "if int(flood): data += ('pane-wide-output ' * 400)\n"
                "data += '\\x1b[0m'\n"
                "os.write(1, data.encode())\n"
                "pathlib.Path(ready).write_text('ready')\n")
            directories = {label: fixture.work / ("project-" + label) for label in ("a", "b", "c")}
            for directory in directories.values():
                directory.mkdir()
            fixture.start()
            panes, rectangles = check_initial_panes(fixture, directories)
            check_protocol_routing(fixture, panes, rectangles)
            check_rendering(fixture, panes, rectangles)
            panes, rectangles = check_resize_zoom(fixture, panes, rectangles)
            panes = check_persistence(fixture, panes, rectangles, directories)
            check_closing(fixture, panes, rectangles)
            check_pending_group_closure(fixture)
            print("ok: native split panes, rendering, input, layouts and closure", flush=True)
        except Exception:
            fixture.log.flush()
            if fixture.app and fixture.app.poll() is None:
                fixture.capture("failure")
            print((fixture.work / "app.log").read_text())
            raise
        finally:
            fixture.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary")
    parser.add_argument("--screenshot")
    args = parser.parse_args()
    if args.binary:
        binary = Path(args.binary).resolve()
    else:
        metadata = json.loads(subprocess.run(["mise", "exec", "--", "cargo", "metadata",
            "--no-deps", "--format-version", "1"], capture_output=True, text=True, check=True).stdout)
        binary = Path(metadata["target_directory"]) / "debug" / "alacritty"
    run(str(binary), args.screenshot)
