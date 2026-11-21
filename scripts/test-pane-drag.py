import argparse
from contextlib import contextmanager
import ctypes
import ctypes.util
import importlib.util
import json
from pathlib import Path
import shlex
import subprocess
import tempfile
import time
import unittest


spec = importlib.util.spec_from_file_location(
    "pane_drag_smoke", Path(__file__).with_name("test-panes.py"))
panes = importlib.util.module_from_spec(spec)
spec.loader.exec_module(panes)
smoke = panes.smoke
BINARY = None
SCREENSHOT = None


@contextmanager
def fixture():
    with tempfile.TemporaryDirectory(prefix="termust-pane-drag-") as directory:
        terminal = panes.Fixture(BINARY, Path(directory), SCREENSHOT)
        try:
            terminal.env["WINIT_X11_SCALE_FACTOR"] = "1.0"
            terminal.start()
            yield terminal
        except Exception:
            terminal.log.flush()
            print((terminal.work / "app.log").read_text()[-5000:], flush=True)
            if terminal.app and terminal.app.poll() is None:
                terminal.capture("pane-drag-failure")
            raise
        finally:
            terminal.close()
            terminal.xvfb.stdout.close()


def move(terminal, column, line):
    _, _, cw, ch = terminal.dimensions()
    terminal.x11.xtest.XTestFakeMotionEvent(terminal.x11.display, -1,
        round((column + 0.5) * cw), round((line + 1.5) * ch), 0)
    terminal.x11.lib.XFlush(terminal.x11.display)
    time.sleep(0.12)


def rectangles(terminal, snapshot=None):
    return panes.layout_rectangles(terminal, snapshot)


def cursor(terminal):
    class Image(ctypes.Structure):
        _fields_ = [
            ("x", ctypes.c_short), ("y", ctypes.c_short),
            ("width", ctypes.c_ushort), ("height", ctypes.c_ushort),
            ("xhot", ctypes.c_ushort), ("yhot", ctypes.c_ushort),
            ("serial", ctypes.c_ulong), ("pixels", ctypes.POINTER(ctypes.c_ulong)),
            ("atom", ctypes.c_ulong), ("name", ctypes.c_char_p),
        ]

    library = ctypes.CDLL(ctypes.util.find_library("Xfixes"))
    library.XFixesGetCursorImage.argtypes = [ctypes.c_void_p]
    library.XFixesGetCursorImage.restype = ctypes.POINTER(Image)
    pointer = library.XFixesGetCursorImage(terminal.x11.display)
    try:
        image = pointer.contents
        return image.name.decode() if image.name else image.serial
    finally:
        terminal.x11.lib.XFree(pointer)


def initialize(terminal, name):
    directory = terminal.work / name
    directory.mkdir(exist_ok=True)
    return terminal.initialize(directory, name)


def setup_split(terminal, nested=False):
    reports = {"a": initialize(terminal, "a")}
    terminal.rename("Source")
    terminal.split("e")
    reports["b"] = initialize(terminal, "b")
    if nested:
        terminal.split("j")
        reports["c"] = initialize(terminal, "c")
    return reports


def new_tab(terminal):
    terminal.x11.chord("Control_L", "Shift_L", "t")
    terminal.rename("Destination")
    return initialize(terminal, "destination")


def select(terminal, index):
    terminal.x11.chord("Alt_L", str(index + 1))
    time.sleep(0.12)


def tab_point(terminal, index, count, create=False):
    columns, _, _, _ = terminal.dimensions()
    width = min(30, max(6, (columns - 6) // count))
    return (width * count + 1 if create else width * index + 3, -1)


def begin(terminal, rect):
    x, y, width, _ = rect
    move(terminal, x + width - 1.5, y)
    terminal.button(True)


def destination(rect, direction):
    x, y, width, height = rect
    return {
        "left": (x, y + height // 2),
        "right": (x + width - 1, y + height // 2),
        "up": (x + width // 2, y),
        "down": (x + width // 2, y + height - 1),
    }[direction]


def assert_shells(test, terminal, reports):
    for name, rect in rectangles(terminal).items():
        terminal.focus_rect(rect)
        report = terminal.probe("survived-" + name)
        test.assertEqual(report["pid"], reports[name]["pid"])
        test.assertEqual(Path(report["cwd"]).name, name)
        test.assertEqual((report["columns"], report["rows"]), rect[2:])


class PaneDragTests(unittest.TestCase):
    def test_hand_cursors_and_handle_after_keyboard_tab_switch(self):
        with fixture() as terminal:
            reports = setup_split(terminal)
            reports["destination"] = new_tab(terminal)
            select(terminal, 0)
            rects = rectangles(terminal)
            move(terminal, 10, 5)
            regular = cursor(terminal)
            x, y, width, _ = rects["b"]
            move(terminal, x + width - 1.5, y)
            hovering = cursor(terminal)
            self.assertNotEqual(hovering, regular)
            if isinstance(hovering, str):
                self.assertIn(hovering, ("grab", "openhand", "hand2"))
            terminal.button(True)
            move(terminal, *destination(rects["a"], "down"))
            dragging = cursor(terminal)
            self.assertNotEqual(dragging, hovering)
            if isinstance(dragging, str):
                self.assertIn(dragging, ("grabbing", "closedhand"))
            terminal.x11.chord("Escape")
            terminal.button(False)
            move(terminal, x + width - 1.5, y)
            select(terminal, 1)
            terminal.button(True)
            move(terminal, *tab_point(terminal, 0, 2))
            terminal.button(False)
            self.assertEqual(len(terminal.snapshot()["tabs"]), 1)
            assert_shells(self, terminal, reports)

    def test_nested_panes_dock_in_all_directions_without_new_shells(self):
        with fixture() as terminal:
            reports = setup_split(terminal, nested=True)
            for direction in ("left", "up", "right", "down"):
                before = rectangles(terminal)
                begin(terminal, before["a"])
                move(terminal, *destination(before["c"], direction))
                if direction == "down":
                    terminal.capture("pane-drag-preview")
                terminal.button(False)
                after = rectangles(terminal)
                a, c = after["a"], after["c"]
                axis = 0 if direction in ("left", "right") else 1
                self.assertTrue(a[axis] < c[axis] if direction in ("left", "up") else a[axis] > c[axis])
                assert_shells(self, terminal, reports)
            terminal.capture("pane-drag-repositioned")

    def test_transfer_join_last_pane_and_detach_keep_original_processes(self):
        with fixture() as terminal:
            reports = setup_split(terminal)
            reports["destination"] = new_tab(terminal)
            select(terminal, 0)
            begin(terminal, rectangles(terminal)["b"])
            move(terminal, *tab_point(terminal, 1, 2))
            terminal.button(False)
            self.assertEqual(terminal.x11.title(terminal.window), "Destination")
            self.assertEqual(len(terminal.snapshot()["tabs"]), 2)
            assert_shells(self, terminal, reports)
            select(terminal, 0)
            begin(terminal, rectangles(terminal)["a"])
            move(terminal, *tab_point(terminal, 1, 2))
            terminal.button(False)
            self.assertEqual(len(terminal.snapshot()["tabs"]), 1)
            assert_shells(self, terminal, reports)
            begin(terminal, rectangles(terminal)["a"])
            move(terminal, *tab_point(terminal, 0, 1, create=True))
            terminal.button(False)
            snapshot = terminal.snapshot()
            self.assertEqual(len(snapshot["tabs"]), 2)
            self.assertEqual(len(panes.saved_panes(snapshot["tabs"][snapshot["active"]])), 1)
            self.assertEqual(terminal.probe("detached-a")["pid"], reports["a"]["pid"])
            select(terminal, 0)
            assert_shells(self, terminal, reports)
            self.assertTrue(all(panes.alive(report["pid"]) for report in reports.values()))
            terminal.capture("pane-drag-transferred")

    def test_hover_tab_allows_docking_and_escape_restores_origin(self):
        with fixture() as terminal:
            reports = setup_split(terminal)
            reports["destination"] = new_tab(terminal)
            before = terminal.snapshot()
            select(terminal, 0)
            source = rectangles(terminal)["b"]
            begin(terminal, source)
            move(terminal, *tab_point(terminal, 1, 2))
            self.assertEqual(terminal.x11.title(terminal.window), "Destination")
            columns, rows, _, _ = terminal.dimensions()
            move(terminal, columns // 2, rows - 1)
            terminal.x11.chord("Escape")
            terminal.button(False)
            self.assertEqual(terminal.x11.title(terminal.window), "Source")
            cancelled = terminal.snapshot()
            self.assertEqual([tab.get("layout") for tab in cancelled["tabs"]],
                [tab.get("layout") for tab in before["tabs"]])
            assert_shells(self, terminal, reports)
            begin(terminal, source)
            move(terminal, *tab_point(terminal, 1, 2))
            move(terminal, columns // 2, rows - 1)
            terminal.capture("pane-drag-other-tab-preview")
            terminal.button(False)
            layout = rectangles(terminal)
            self.assertGreater(layout["b"][1], layout["destination"][1])
            assert_shells(self, terminal, reports)

    def test_running_mouse_application_receives_no_drag_input(self):
        with fixture() as terminal:
            reports = setup_split(terminal)
            raw = terminal.work / "raw.py"
            raw.write_text("import os, pathlib, select, termios, time, tty\n"
                "root = pathlib.Path(__file__).parent\n"
                "old = termios.tcgetattr(0)\n"
                "data = bytearray()\n"
                "try:\n"
                "    tty.setraw(0)\n"
                "    os.write(1, b'\\x1b[?1000h\\x1b[?1006h')\n"
                "    (root / 'ready').touch()\n"
                "    while not (root / 'gate').exists():\n"
                "        if select.select([0], [], [], 0.03)[0]: data.extend(os.read(0, 4096))\n"
                "finally:\n"
                "    os.write(1, b'\\x1b[?1000l\\x1b[?1006l')\n"
                "    termios.tcsetattr(0, termios.TCSANOW, old)\n"
                "    (root / 'received').write_bytes(data)\n")
            before = rectangles(terminal)
            terminal.command("python3 " + shlex.quote(str(raw)))
            smoke.wait_for((terminal.work / "ready").exists, "raw mouse app ready")
            begin(terminal, before["b"])
            move(terminal, *destination(before["a"], "down"))
            terminal.x11.chord("Escape")
            terminal.button(False)
            begin(terminal, before["b"])
            move(terminal, *destination(before["a"], "down"))
            terminal.button(False)
            (terminal.work / "gate").touch()
            smoke.wait_for((terminal.work / "received").exists, "moved app finished")
            self.assertEqual((terminal.work / "received").read_bytes(), b"")
            assert_shells(self, terminal, reports)

    def test_focus_loss_and_disappearing_source_cancel_safely(self):
        with fixture() as terminal:
            reports = setup_split(terminal)
            before = terminal.snapshot()["tabs"][0]["layout"]
            rects = rectangles(terminal)
            begin(terminal, rects["b"])
            bx, by, bw, bh = rects["b"]
            move(terminal, bx + bw // 2, by + bh // 2)
            terminal.button(False)
            self.assertEqual(terminal.snapshot()["tabs"][0]["layout"], before)
            begin(terminal, rects["b"])
            move(terminal, *destination(rects["a"], "down"))
            terminal.x11.lib.XSetInputFocus(terminal.x11.display, 0, 2, 0)
            terminal.x11.lib.XFlush(terminal.x11.display)
            time.sleep(0.15)
            terminal.x11.lib.XSetInputFocus(terminal.x11.display, terminal.window, 2, 0)
            terminal.x11.lib.XFlush(terminal.x11.display)
            terminal.button(False)
            self.assertEqual(terminal.snapshot()["tabs"][0]["layout"], before)
            assert_shells(self, terminal, reports)
            terminal.focus_rect(rects["b"])
            gate = terminal.work / "exit-gate"
            ready = terminal.work / "exit-ready"
            terminal.command(f"touch {shlex.quote(str(ready))}; while test ! -e "
                f"{shlex.quote(str(gate))}; do sleep 0.03; done; exit")
            smoke.wait_for(ready.exists, "source waits for exit")
            begin(terminal, rects["b"])
            move(terminal, *destination(rects["a"], "down"))
            gate.touch()
            smoke.wait_for(lambda: not panes.alive(reports["b"]["pid"]), "source exited during drag")
            terminal.button(False)
            self.assertIsNone(terminal.snapshot()["tabs"][0].get("layout"))
            self.assertEqual(terminal.probe("exit-survivor")["pid"], reports["a"]["pid"])

    def test_moved_layout_and_output_restore_from_workspace(self):
        with fixture() as terminal:
            reports = setup_split(terminal, nested=True)
            rects = rectangles(terminal)
            begin(terminal, rects["a"])
            move(terminal, *destination(rects["c"], "down"))
            terminal.button(False)
            terminal.command("printf 'PANE-MOVE-PRESERVED-781\\n'")
            before = terminal.snapshot()["tabs"][0]
            self.assertIn("PANE-MOVE-PRESERVED-781", "".join(record.get("screen_text", "")
                for record in panes.saved_panes(before)))
            terminal.save_workspace("Moved")
            terminal.general(restore_session=True, startup_session="Moved")
            terminal.stop()
            terminal.start()
            restored = terminal.snapshot()["tabs"][0]
            self.assertEqual(restored["layout"], before["layout"])
            self.assertEqual(restored["active_pane"], before["active_pane"])
            self.assertIn("PANE-MOVE-PRESERVED-781", "".join(record.get("screen_text", "")
                for record in panes.saved_panes(restored)))
            for name, rect in rectangles(terminal).items():
                terminal.focus_rect(rect)
                current = terminal.probe("restored-" + name)
                self.assertEqual(Path(current["cwd"]).name, name)
                self.assertNotEqual(current["pid"], reports[name]["pid"])


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary")
    parser.add_argument("--screenshot")
    parser.add_argument("--case", action="append")
    args = parser.parse_args()
    if args.binary:
        BINARY = str(Path(args.binary).resolve())
    else:
        metadata = json.loads(subprocess.check_output(["mise", "exec", "--", "cargo",
            "metadata", "--no-deps", "--format-version", "1"], text=True))
        BINARY = str(Path(metadata["target_directory"]) / "debug/termust")
    SCREENSHOT = args.screenshot
    suite = unittest.TestSuite(PaneDragTests(name) for name in args.case) if args.case else (
        unittest.defaultTestLoader.loadTestsFromTestCase(PaneDragTests))
    raise SystemExit(not unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful())
