import argparse
from contextlib import contextmanager
import importlib.util
import json
from pathlib import Path
import shlex
import subprocess
import tempfile
import time
import unittest


module = importlib.util.spec_from_file_location(
    "productivity_smoke", Path(__file__).with_name("test-productivity.py"))
productivity = importlib.util.module_from_spec(module)
module.loader.exec_module(productivity)
smoke = productivity.smoke
BINARY = None
SCREENSHOT = None
RENAME_MENU_ROW = 7


@contextmanager
def fixture(config=""):
    with tempfile.TemporaryDirectory(prefix="alacritty-shortcuts-") as directory:
        terminal = productivity.Fixture(BINARY, Path(directory), SCREENSHOT)
        try:
            terminal.config.write_text(terminal.config.read_text() + config)
            terminal.start()
            yield terminal
        except Exception:
            terminal.log.flush()
            print((terminal.work / "app.log").read_text()[-3000:], flush=True)
            raise
        finally:
            terminal.close()


@contextmanager
def raw_input(terminal, label, alternate=False):
    source = terminal.work / f"{label}.py"
    captured = terminal.work / f"{label}.input"
    ready = terminal.work / f"{label}.ready"
    finish = terminal.work / f"{label}.finish"
    done = terminal.work / f"{label}.done"
    source.write_text('''import os
from pathlib import Path
import select
import sys
import termios
import time
import tty

captured, ready, finish, done = map(Path, sys.argv[1:5])
alternate = sys.argv[5] == "1"
fd = sys.stdin.fileno()
original = termios.tcgetattr(fd)
data = bytearray()
captured.write_bytes(data)
try:
    tty.setraw(fd)
    if alternate:
        os.write(sys.stdout.fileno(), b"\\x1b[?1049h")
    ready.touch()
    deadline = time.monotonic() + 30
    while not finish.exists() and time.monotonic() < deadline:
        readable, _, _ = select.select([fd], [], [], 0.05)
        if readable:
            chunk = os.read(fd, 4096)
            if not chunk:
                break
            data.extend(chunk)
            captured.write_bytes(data)
finally:
    if alternate:
        os.write(sys.stdout.fileno(), b"\\x1b[?1049l")
    termios.tcsetattr(fd, termios.TCSANOW, original)
    done.touch()
''')
    terminal.command("python3 " + " ".join(shlex.quote(str(argument)) for argument in
        (source, captured, ready, finish, done, int(alternate))))
    smoke.wait_for(ready.exists, f"{label} raw PTY ready")
    time.sleep(0.15)
    try:
        yield captured
    finally:
        finish.touch()
        smoke.wait_for(done.exists, f"{label} raw PTY restored")


def rename(terminal, name, *keys):
    terminal.x11.chord(*keys)
    apply_name(terminal, name)


def apply_name(terminal, name):
    terminal.x11.chord("Control_L", "a")
    terminal.x11.type(name)
    terminal.x11.chord("Return")
    smoke.wait_for(lambda: terminal.x11.title(terminal.window) == name,
        f"tab renamed to {name}")


def assert_shell_ready(terminal, label):
    marker = terminal.work / label
    terminal.command(f"touch {shlex.quote(str(marker))}")
    smoke.wait_for(marker.exists, f"{label} reached shell without an open dialog")


def shortcut_pixels(terminal, label):
    left, top = terminal.menu()
    path = terminal.capture(label)
    pixels = terminal.pixels(path, left + 25, top + RENAME_MENU_ROW, 23)
    terminal.x11.chord("Escape")
    return terminal.text_mask(pixels)


class ShortcutTests(unittest.TestCase):
    def test_rename_and_tab_navigation(self):
        with fixture() as terminal:
            with raw_input(terminal, "normal") as captured:
                rename(terminal, "First", "F2")
                time.sleep(0.15)
                self.assertEqual(captured.read_bytes(), b"",
                    "F2 rename also sent a function key or dialog text to the shell")
            assert_shell_ready(terminal, "after-f2")
            for name in ("Second", "Third"):
                terminal.x11.chord("Control_L", "Shift_L", "t")
                rename(terminal, name, "F2")
            for keys, name in [
                (("Control_L", "Tab"), "First"),
                (("Control_L", "Tab"), "Second"),
                (("Control_L", "Shift_L", "Tab"), "First"),
                (("Control_L", "Shift_L", "Tab"), "Third"),
                (("Control_L", "Page_Up"), "Second"),
                (("Control_L", "Page_Down"), "Third"),
            ]:
                terminal.x11.chord(*keys)
                smoke.wait_for(lambda: terminal.x11.title(terminal.window) == name,
                    f"{'+'.join(keys)} selected {name}")
            rename(terminal, "Legacy rename", "Control_L", "Shift_L", "r")

    def test_alternate_screen_preserves_f2_and_legacy_rename(self):
        with fixture() as terminal:
            with raw_input(terminal, "alternate", alternate=True) as captured:
                terminal.x11.chord("F2")
                smoke.wait_for(lambda: captured.read_bytes() == b"\x1bOQ",
                    "alternate-screen app received F2")
                rename(terminal, "Alternate app", "Control_L", "Shift_L", "r")
                time.sleep(0.15)
                self.assertEqual(captured.read_bytes(), b"\x1bOQ")
            assert_shell_ready(terminal, "after-alternate")
            rename(terminal, "Normal again", "F2")

    def test_user_keyboard_binding_takes_priority(self):
        with fixture('\n[keyboard]\nbindings = [{ key = "F2", chars = "CUSTOM" }]\n') as terminal:
            with raw_input(terminal, "custom-binding") as captured:
                terminal.x11.chord("F2")
                smoke.wait_for(lambda: captured.read_bytes() == b"CUSTOM",
                    "user F2 binding reached the PTY exactly once")
            assert_shell_ready(terminal, "after-custom-binding")
            rename(terminal, "Legacy available", "Control_L", "Shift_L", "r")

    def test_custom_and_disabled_rename_match_menu(self):
        with fixture() as terminal:
            original = terminal.config.read_text()
            default = shortcut_pixels(terminal, "default-menu")
            self.assertTrue(any(default), "Default rename shortcut missing from menu")

            def configure(value):
                terminal.config.write_text(original +
                    f'\n[general.shortcuts]\nRenameTab = {json.dumps(value)}\n')
                smoke.wait_for(lambda: terminal.runtime_config()["general"]["shortcuts"].get(
                    "RenameTab") == value, f"rename shortcut reloaded as {value!r}")

            configure("F8")
            custom = shortcut_pixels(terminal, "custom-menu")
            self.assertTrue(any(custom), "Custom rename shortcut missing from menu")
            self.assertFalse(terminal.same_text(default, custom),
                "Menu kept the default shortcut after customization")
            rename(terminal, "Custom shortcut", "F8")
            with raw_input(terminal, "custom-old-key") as captured:
                terminal.x11.chord("F2")
                smoke.wait_for(lambda: captured.read_bytes() == b"\x1bOQ",
                    "old F2 shortcut returned to the application")
            assert_shell_ready(terminal, "after-custom-shortcut")

            configure("")
            disabled = shortcut_pixels(terminal, "disabled-menu")
            self.assertFalse(any(disabled), "Disabled rename still has a menu shortcut")
            with raw_input(terminal, "disabled") as captured:
                terminal.x11.chord("F2")
                terminal.x11.chord("F8")
                smoke.wait_for(lambda: captured.read_bytes() == b"\x1bOQ\x1b[19~",
                    "disabled rename keys reached the application")
            assert_shell_ready(terminal, "after-disabled-shortcut")
            left, top = terminal.menu()
            terminal.click(left + 5, top + RENAME_MENU_ROW)
            apply_name(terminal, "Menu still works")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--screenshot", type=Path)
    parser.add_argument("--case", action="append", choices=[name for name in dir(ShortcutTests)
        if name.startswith("test_")])
    args = parser.parse_args()
    if args.binary:
        BINARY = str(args.binary.resolve())
    else:
        metadata = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"], text=True))
        BINARY = str(Path(metadata["target_directory"]) / "debug" / "termust")
    SCREENSHOT = args.screenshot
    suite = unittest.TestSuite(ShortcutTests(name) for name in args.case) if args.case else (
        unittest.defaultTestLoader.loadTestsFromTestCase(ShortcutTests))
    raise SystemExit(not unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful())
