import argparse
from contextlib import contextmanager
import ctypes
import importlib.util
import json
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import time
import unittest

spec = importlib.util.spec_from_file_location("productivity", Path(__file__).with_name("test-productivity.py"))
productivity = importlib.util.module_from_spec(spec)
spec.loader.exec_module(productivity)
smoke = productivity.smoke
ROOT = Path(__file__).resolve().parents[1]
BINARY = None
SCREENSHOT = None


@contextmanager
def fixture():
    with tempfile.TemporaryDirectory(prefix="termust-branding-") as directory:
        terminal = productivity.Fixture(BINARY, Path(directory), SCREENSHOT)
        try:
            yield terminal
        except Exception:
            terminal.log.flush()
            print((terminal.work / "app.log").read_text()[-3000:], flush=True)
            raise
        finally:
            terminal.close()
            terminal.xvfb.stdout.close()


def config_at(terminal, name, size):
    path = terminal.work / "config" / name / f"{name}.toml"
    path.parent.mkdir(parents=True, exist_ok=True)
    contents = terminal.config.read_text().replace('title = "language-ui"\n', "")
    contents = contents.replace('import = ["import.toml"]', "")
    path.write_text(contents + f"\n[font]\nsize = {size}\n")
    return path


def saved_session(title, directory):
    return {
        "version": 1, "active": 0,
        "tabs": [{
            "title": title, "working_directory": str(directory), "commands": [],
            "replay_last_command": False,
            "shell": {"program": "/bin/bash", "args": ["--noprofile", "--norc"]},
        }],
    }


def state_at(terminal, name, title):
    path = terminal.work / "state" / name / "tabs.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(saved_session(title, terminal.work)))
    return path


def start(terminal, title):
    terminal.app = subprocess.Popen([terminal.binary, "--socket", str(terminal.socket)],
        env=dict(terminal.env, LANG="C", XDG_CONFIG_DIRS=str(terminal.work / "system-config")),
        stdout=terminal.log, stderr=terminal.log)
    terminal.window = smoke.wait_for(lambda: next((window for window in terminal.x11.windows()
        if terminal.x11.title(window) == title), None), f"Termust opened with {title}")
    time.sleep(0.25)
    terminal.x11.lib.XSetInputFocus(terminal.x11.display, terminal.window, 2, 0)
    terminal.x11.lib.XFlush(terminal.x11.display)
    terminal.command(f"stty size > {shlex.quote(str(terminal.size))}")
    smoke.wait_for(terminal.size.exists, "Termust shell initialized")
    return terminal.runtime_config()


def property_value(terminal, name):
    x11 = terminal.x11
    x11.bind("XInternAtom", [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int], ctypes.c_ulong)
    x11.bind("XGetWindowProperty", [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_ulong,
        ctypes.c_long, ctypes.c_long, ctypes.c_int, ctypes.c_ulong,
        ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_int),
        ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_ulong),
        ctypes.POINTER(ctypes.POINTER(ctypes.c_ubyte))])
    atom = x11.lib.XInternAtom(x11.display, name.encode(), False)
    actual, count, remaining = ctypes.c_ulong(), ctypes.c_ulong(), ctypes.c_ulong()
    bits = ctypes.c_int()
    data = ctypes.POINTER(ctypes.c_ubyte)()
    result = x11.lib.XGetWindowProperty(x11.display, terminal.window, atom,
        0, 1024 * 1024, False, 0, ctypes.byref(actual), ctypes.byref(bits),
        ctypes.byref(count), ctypes.byref(remaining), ctypes.byref(data))
    if result or remaining.value or not data:
        raise AssertionError(f"Cannot read complete X11 property {name}")
    try:
        if bits.value == 8:
            return bytes(data[:count.value])
        if bits.value == 32:
            return [value & 0xffffffff for value in
                ctypes.cast(data, ctypes.POINTER(ctypes.c_ulong))[:count.value]]
        raise AssertionError(f"Unexpected property format for {name}: {bits.value}")
    finally:
        x11.lib.XFree(data)


class BrandingTests(unittest.TestCase):
    def test_native_identity_and_embedded_logo(self):
        self.assertTrue(subprocess.check_output([BINARY, "--version"], text=True).startswith("termust "))
        help_text = subprocess.check_output([BINARY, "--help"], text=True)
        self.assertIn("termust", help_text.lower())
        with fixture() as terminal:
            terminal.config = config_at(terminal, "termust", 13)
            config = start(terminal, "Termust")
            self.assertEqual(config["font"]["size"], 13)
            self.assertEqual(terminal.x11.raw_title(terminal.window), "Termust")
            self.assertEqual(property_value(terminal, "WM_CLASS"), b"Termust\0Termust\0")
            icon = property_value(terminal, "_NET_WM_ICON")
            self.assertEqual(icon[:2], [256, 256])
            rgba = subprocess.check_output([shutil.which("magick") or "convert", str(ROOT / "extra/logo/termust.png"),
                "-depth", "8", "rgba:-"])
            expected = [(alpha << 24) | (red << 16) | (green << 8) | blue
                for red, green, blue, alpha in zip(rgba[0::4], rgba[1::4], rgba[2::4], rgba[3::4])]
            self.assertEqual(icon[2:], expected)
            probe = terminal.work / "environment"
            terminal.command("printf '%s\\n' \"$TERMUST_WINDOW_ID\" \"$ALACRITTY_WINDOW_ID\" "
                f'\"$TERMUST_SOCKET\" \"$TERM\" > {shlex.quote(str(probe))}')
            smoke.wait_for(probe.exists, "Termust environment reached its shell")
            window, legacy_window, socket, term = probe.read_text().splitlines()
            self.assertEqual(window, str(terminal.window))
            self.assertEqual(window, legacy_window)
            self.assertEqual(socket, str(terminal.socket))
            self.assertIn(term, ["alacritty", "xterm-256color"])
            terminal.command("clear; printf '\\n  Termust\\n  Rust terminal with tabs and split panes.\\n'")
            terminal.capture("termust-window")
            left, top = terminal.menu()
            self.assertGreater(left, 0)
            self.assertGreaterEqual(top, 0)
            terminal.capture("termust-menu")

    def test_legacy_configuration_and_session_are_preserved(self):
        with fixture() as terminal:
            terminal.config = config_at(terminal, "alacritty", 14)
            old_config = terminal.config.read_bytes()
            legacy = state_at(terminal, "alacritty", "Legacy project")
            old_session = legacy.read_bytes()
            config = start(terminal, "Legacy project")
            self.assertEqual(config["font"]["size"], 14)
            self.assertEqual(terminal.x11.raw_title(terminal.window), "Legacy project — Termust")
            terminal.rename("Termust project")
            snapshot = terminal.snapshot()
            self.assertEqual(snapshot["tabs"][0]["title"], "Termust project")
            self.assertTrue((terminal.work / "state/termust/tabs.json").exists())
            self.assertEqual(legacy.read_bytes(), old_session)
            self.assertEqual(terminal.config.read_bytes(), old_config)

    def test_termust_configuration_and_session_take_precedence(self):
        with fixture() as terminal:
            legacy_config = config_at(terminal, "alacritty", 10)
            legacy_state = state_at(terminal, "alacritty", "Legacy project")
            original = (legacy_config.read_bytes(), legacy_state.read_bytes())
            terminal.config = config_at(terminal, "termust", 15)
            state_at(terminal, "termust", "Termust project")
            config = start(terminal, "Termust project")
            self.assertEqual(config["font"]["size"], 15)
            self.assertEqual(terminal.x11.raw_title(terminal.window), "Termust project — Termust")
            self.assertEqual((legacy_config.read_bytes(), legacy_state.read_bytes()), original)


    def test_caption_follows_shell_and_custom_names_without_polluting_saved_titles(self):
        with fixture() as terminal:
            terminal.config = config_at(terminal, "termust", 13)
            terminal.config.write_text(terminal.config.read_text().replace(
                "dynamic_title = false", "dynamic_title = true"))
            start(terminal, "Termust")
            terminal.command("printf '\\033]0;Shell project\\007'")
            smoke.wait_for(lambda: terminal.x11.raw_title(terminal.window) ==
                "Shell project — Termust", "shell caption keeps the Termust identity")
            terminal.rename("Named project")
            self.assertEqual(terminal.x11.raw_title(terminal.window), "Named project — Termust")
            terminal.command("printf '\\033]0;Background shell\\007'")
            time.sleep(.2)
            self.assertEqual(terminal.x11.raw_title(terminal.window), "Named project — Termust")
            self.assertEqual(terminal.snapshot()["tabs"][0]["title"], "Named project")
            terminal.x11.chord("F2")
            terminal.x11.chord("Control_L", "a")
            terminal.x11.chord("BackSpace")
            terminal.x11.chord("Return")
            smoke.wait_for(lambda: terminal.x11.raw_title(terminal.window) ==
                "Background shell — Termust", "clearing a custom name restores the shell caption")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--screenshot", type=Path)
    args = parser.parse_args()
    if args.binary:
        BINARY = str(args.binary.resolve())
    else:
        metadata = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"], text=True))
        BINARY = str(Path(metadata["target_directory"]) / "debug/termust")
    SCREENSHOT = args.screenshot
    unittest.main(argv=[__file__], verbosity=2)
