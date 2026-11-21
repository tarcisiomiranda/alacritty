import ast
import hashlib
import json
import os
from pathlib import Path
import platform
import signal
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parent.parent
BINARY = Path(os.environ.get("TERMUST_BINARY", ROOT / "target/debug/termust"))
HELPER_DIR = ROOT / "target/termust-macos-gui"
TAB_ROW_Y = 34
TITLEBAR_STRIP_Y = 12
KEY = {
    "return": 36, "tab": 48, "escape": 53, "f2": 120, "end": 119, "up": 126,
    "left_bracket": 33, "right_bracket": 30,
}
MOUSE_SOURCE = r"""
import CoreGraphics
import Foundation
let args = CommandLine.arguments
let source = CGEventSource(stateID: .hidSystemState)
func post(_ type: CGEventType, _ x: Double, _ y: Double, _ button: CGMouseButton) {
    CGEvent(mouseEventSource: source, mouseType: type, mouseCursorPosition: CGPoint(x: x, y: y), mouseButton: button)?.post(tap: .cghidEventTap)
    usleep(60000)
}
let x = Double(args[2])!, y = Double(args[3])!
switch args[1] {
case "click":
    post(.mouseMoved, x, y, .left); post(.leftMouseDown, x, y, .left); post(.leftMouseUp, x, y, .left)
case "right":
    post(.mouseMoved, x, y, .right); post(.rightMouseDown, x, y, .right); post(.rightMouseUp, x, y, .right)
default:
    exit(2)
}
"""


def mouse_helper():
    digest = hashlib.sha256(MOUSE_SOURCE.encode()).hexdigest()[:16]
    helper = HELPER_DIR / f"mouse-{digest}"
    if not helper.is_file():
        HELPER_DIR.mkdir(parents=True, exist_ok=True)
        source = HELPER_DIR / f"mouse-{digest}.swift"
        source.write_text(MOUSE_SOURCE)
        subprocess.run(["xcrun", "swiftc", "-O", str(source), "-o", str(helper)], check=True,
            capture_output=True)
    return helper


def fake_agent_source():
    module = ast.parse((Path(__file__).with_name("test-agents.py")).read_text())
    for node in module.body:
        if isinstance(node, ast.Assign) and any(getattr(target, "id", None) == "FAKE"
                for target in node.targets):
            return ast.literal_eval(node.value)
    raise RuntimeError("test-agents.py no longer defines FAKE")


def read_json(path):
    try:
        return json.loads(Path(path).read_text())
    except (OSError, ValueError):
        return None


def wait_for(predicate, message, timeout=12.0, interval=0.3):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        last = predicate()
        if last:
            return last
        time.sleep(interval)
    raise AssertionError(f"timed out waiting for {message}; last value: {last!r}")


class Terminal:
    def __init__(self, root, window="", general="", isolated_shell=True):
        self.root = root
        self.home = root / "home"
        self.state_dir = root / "state/termust"
        self.process = None
        self.log = None
        self.home.mkdir(parents=True, exist_ok=True)
        (self.home / ".zshrc").write_text("")
        config_dir = root / "config/termust"
        config_dir.mkdir(parents=True, exist_ok=True)
        shell = '[terminal.shell]\nprogram = "/bin/zsh"\nargs = ["-i"]\n' if isolated_shell else ""
        (config_dir / "termust.toml").write_text(
            "[window]\nposition = { x = 160, y = 160 }\ndimensions = { columns = 100, lines = 28 }\n"
            f"{window}[general]\nsession_autosave = true\nlanguage = \"en\"\n{general}{shell}")
        self.env = dict(os.environ, XDG_CONFIG_HOME=str(root / "config"),
            XDG_STATE_HOME=str(root / "state"))
        if isolated_shell:
            self.env["HOME"] = str(self.home)
        self.mouse = mouse_helper()

    def start(self):
        if self.log is None:
            self.log = (self.root / "stderr.txt").open("a")
        self.process = subprocess.Popen([str(BINARY)], env=self.env, cwd=self.home,
            stdout=subprocess.DEVNULL, stderr=self.log, start_new_session=True)
        wait_for(lambda: self.alive() and self.windows() >= 1, "the first window")
        self.focus()
        return self

    def alive(self):
        return self.process is not None and self.process.poll() is None

    def stop(self):
        if self.alive():
            self.process.terminate()
            try:
                self.process.wait(5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(5)
        if self.log is not None:
            self.log.close()
            self.log = None

    def crash(self):
        os.kill(self.process.pid, signal.SIGKILL)
        self.process.wait(5)

    def osa(self, body):
        script = ('tell application "System Events"\n'
            f" set p to first process whose unix id is {self.process.pid}\n{body}\nend tell")
        result = subprocess.run(["osascript", "-e", script], capture_output=True, text=True)
        if result.returncode != 0:
            raise RuntimeError(result.stderr.strip())
        return result.stdout.strip()

    def windows(self):
        try:
            return int(self.osa(" return count windows of p"))
        except (RuntimeError, ValueError):
            return 0

    def focus(self):
        def frontmost():
            self.osa(" set frontmost of p to true")
            return self.osa(" return frontmost of p") == "true"
        wait_for(frontmost, "Termust to become the frontmost application", timeout=8)

    def keystroke(self, text, *modifiers, settle=0.6):
        self.focus()
        using = " using {" + ", ".join(f"{name} down" for name in modifiers) + "}" if modifiers else ""
        self.osa(f' keystroke "{text}"{using}')
        time.sleep(settle)

    def key(self, name, *modifiers, settle=0.6):
        self.focus()
        using = " using {" + ", ".join(f"{name} down" for name in modifiers) + "}" if modifiers else ""
        self.osa(f" key code {KEY[name]}{using}")
        time.sleep(settle)

    def type_line(self, text):
        self.keystroke(text, settle=0.2)
        self.key("return")

    def frame(self):
        values = self.osa(" tell p to get {position, size} of window 1")
        return [float(value) for value in values.split(",")]

    def click(self, x, y, button="click"):
        self.focus()
        subprocess.run([str(self.mouse), button, str(x), str(y)], check=True)
        time.sleep(0.6)

    def click_tab_bar(self, fraction, y=TAB_ROW_Y):
        left, top, width, _ = self.frame()
        self.click(left + width * fraction, top + y)

    def open_menu(self):
        left, top, width, height = self.frame()
        self.click(left + width / 2, top + height / 2, "right")

    def snapshot(self):
        path = self.state_dir / "recovery.json"
        try:
            return json.loads(path.read_text())
        except (OSError, ValueError):
            return None

    def wait_state(self, predicate, message, timeout=12.0):
        def check():
            snapshot = self.snapshot()
            return snapshot if snapshot and predicate(snapshot) else None
        return wait_for(check, message, timeout)

    def shells(self):
        result = subprocess.run(["pgrep", "-P", str(self.process.pid)], capture_output=True, text=True)
        return [int(pid) for pid in result.stdout.split()]


def tab_titles(snapshot):
    return [tab.get("title") for tab in snapshot["tabs"]]


def pane_count(tab):
    return 1 + len(tab.get("panes", []))


@unittest.skipUnless(platform.system() == "Darwin", "macOS graphical checks")
class MacGui(unittest.TestCase):
    def setUp(self):
        if not BINARY.is_file():
            self.skipTest(f"build {BINARY} first")
        self.directory = tempfile.TemporaryDirectory(prefix="termust-macos-gui-")
        self.root = Path(self.directory.name)
        self.terminals = []

    def tearDown(self):
        for terminal in self.terminals:
            terminal.stop()
        self.directory.cleanup()

    def terminal(self, name, window="", general="", isolated_shell=True):
        terminal = Terminal(self.root / name, window, general, isolated_shell)
        self.terminals.append(terminal)
        return terminal.start()

    def rename(self, terminal, name):
        terminal.key("f2")
        terminal.keystroke("a", "command")
        terminal.keystroke(name, settle=0.2)
        terminal.key("return")

    def test_command_shortcuts_drive_internal_tabs(self):
        terminal = self.terminal("tabs")
        terminal.wait_state(lambda s: len(s["tabs"]) == 1, "one tab")
        for _ in range(2):
            terminal.keystroke("t", "command")
        terminal.wait_state(lambda s: len(s["tabs"]) == 3 and s["active"] == 2, "three tabs")
        self.assertEqual(terminal.windows(), 1)
        steps = [
            (lambda: terminal.keystroke("1", "command"), 0),
            (lambda: terminal.keystroke("9", "command"), 2),
            (lambda: terminal.key("tab", "control"), 0),
            (lambda: terminal.key("tab", "control", "shift"), 2),
            (lambda: terminal.key("right_bracket", "command", "shift"), 0),
            (lambda: terminal.key("left_bracket", "command", "shift"), 2),
            (lambda: terminal.keystroke("2", "command"), 1),
        ]
        for action, expected in steps:
            action()
            terminal.wait_state(lambda s, expected=expected: s["active"] == expected,
                f"active tab {expected}")
        terminal.keystroke("w", "command")
        terminal.keystroke("n")
        time.sleep(2.5)
        self.assertEqual(len(terminal.snapshot()["tabs"]), 3)
        terminal.keystroke("w", "command")
        terminal.key("return")
        terminal.wait_state(lambda s: len(s["tabs"]) == 2, "tab closed after confirmation")
        terminal.keystroke("t", "command", "shift")
        terminal.wait_state(lambda s: len(s["tabs"]) == 3, "closed tab reopened")
        wait_for(lambda: len(terminal.shells()) == 3, "one shell per tab")

    def test_panes_split_and_close_with_confirmation(self):
        terminal = self.terminal("panes")
        terminal.wait_state(lambda s: len(s["tabs"]) == 1, "one tab")
        terminal.keystroke("d", "command")
        terminal.wait_state(lambda s: pane_count(s["tabs"][0]) == 2, "vertical split")
        terminal.keystroke("d", "command", "shift")
        terminal.wait_state(lambda s: pane_count(s["tabs"][0]) == 3, "horizontal split")
        terminal.keystroke("w", "command", "option")
        terminal.key("return")
        terminal.wait_state(lambda s: pane_count(s["tabs"][0]) == 2, "pane closed")
        self.assertEqual(len(terminal.snapshot()["tabs"]), 1)

    def test_new_window_and_last_tab_close(self):
        terminal = self.terminal("windows")
        terminal.keystroke("n", "command")
        wait_for(lambda: terminal.windows() == 2, "second window")
        terminal.keystroke("w", "command")
        terminal.key("return")
        wait_for(lambda: terminal.windows() == 1, "second window closed by its last tab")
        self.assertTrue(terminal.alive())

    def test_quit_asks_once_for_every_window(self):
        terminal = self.terminal("quit")
        terminal.keystroke("t", "command")
        terminal.keystroke("n", "command")
        wait_for(lambda: terminal.windows() == 2, "second window")
        terminal.keystroke("q", "command")
        time.sleep(1)
        self.assertTrue(terminal.alive())
        terminal.keystroke("n")
        self.assertEqual(terminal.windows(), 2)
        terminal.focus()
        terminal.osa(' tell p to click (first menu item of menu 1 of menu bar item 2 of menu bar 1'
            ' whose name starts with "Quit")')
        time.sleep(1)
        self.assertTrue(terminal.alive())
        terminal.key("escape")
        self.assertEqual(terminal.windows(), 2)
        terminal.keystroke("q", "command")
        terminal.key("return", settle=0.2)
        wait_for(lambda: not terminal.alive(), "Termust to quit after confirmation")

    def test_quit_without_confirmation_closes_immediately(self):
        terminal = self.terminal("quit-now", general="confirm_tab_close = false\n")
        terminal.keystroke("n", "command")
        wait_for(lambda: terminal.windows() == 2, "second window")
        terminal.keystroke("q", "command", settle=0.2)
        wait_for(lambda: not terminal.alive(), "Termust to quit")

    def test_groups_are_created_from_the_menu_and_survive_a_crash(self):
        terminal = self.terminal("groups")
        terminal.keystroke("t", "command")
        terminal.wait_state(lambda s: len(s["tabs"]) == 2, "two tabs")
        terminal.open_menu()
        for name in ("end", "up", "up", "return"):
            terminal.key(name, settle=0.3)
        terminal.keystroke("a", "command")
        terminal.keystroke("Work", settle=0.2)
        terminal.key("tab", settle=0.3)
        terminal.key("return")
        snapshot = terminal.wait_state(lambda s: s.get("active_group") == "Work", "group Work")
        self.assertEqual([tab.get("group") for tab in snapshot["tabs"]], [None, "Work"])
        terminal.keystroke("t", "command")
        snapshot = terminal.wait_state(lambda s: len(s["tabs"]) == 3, "tab inside the group")
        self.assertEqual(snapshot["tabs"][2].get("group"), "Work")
        terminal.crash()
        terminal.start()
        restored = terminal.wait_state(lambda s: len(s["tabs"]) == 3, "restored groups")
        self.assertEqual(restored.get("active_group"), "Work")
        self.assertEqual([tab.get("group") for tab in restored["tabs"]], [None, "Work", "Work"])

    def test_recovery_after_unexpected_exit_restores_names_folders_and_text(self):
        terminal = self.terminal("recovery")
        folder = terminal.home / "projeto-recovery"
        folder.mkdir()
        terminal.wait_state(lambda s: len(s["tabs"]) == 1, "one tab")
        self.rename(terminal, "api")
        terminal.type_line(f"cd {folder}")
        terminal.type_line("echo termust-recovery-marker")
        terminal.keystroke("t", "command")
        self.rename(terminal, "web")
        terminal.keystroke("d", "command")
        terminal.keystroke("1", "command")

        def ready(snapshot):
            return (tab_titles(snapshot) == ["api", "web"] and snapshot["active"] == 0
                and Path(snapshot["tabs"][0]["working_directory"] or "").resolve() == folder.resolve()
                and "termust-recovery-marker" in snapshot["tabs"][0].get("screen_text", "")
                and pane_count(snapshot["tabs"][1]) == 2)
        saved = terminal.wait_state(ready, "recovery snapshot before the crash")
        self.assertTrue(any("echo termust-recovery-marker" in command.get("command", "")
            for command in saved["tabs"][0]["commands"]))
        terminal.crash()
        terminal.start()
        restored = terminal.wait_state(lambda s: tab_titles(s) == ["api", "web"], "restored tabs")
        self.assertEqual(Path(restored["tabs"][0]["working_directory"]).resolve(), folder.resolve())
        self.assertIn("termust-recovery-marker", restored["tabs"][0].get("screen_text", ""))
        self.assertEqual(pane_count(restored["tabs"][1]), 2)
        wait_for(lambda: len(terminal.shells()) == 3, "fresh shells for every pane")

    def test_titlebar_strip_never_selects_tabs(self):
        for decorations in ("Full", "Transparent", "Buttonless"):
            with self.subTest(decorations=decorations):
                terminal = self.terminal(f"titlebar-{decorations}",
                    window=f'decorations = "{decorations}"\n', general="confirm_tab_close = false\n")
                terminal.keystroke("t", "command")
                terminal.wait_state(lambda s: len(s["tabs"]) == 2 and s["active"] == 1, "two tabs")
                terminal.click_tab_bar(0.15, TITLEBAR_STRIP_Y)
                time.sleep(2.5)
                self.assertEqual(terminal.snapshot()["active"], 1)
                terminal.click_tab_bar(0.15)
                terminal.wait_state(lambda s: s["active"] == 0, "first tab selected")
                terminal.stop()

    def test_agent_permission_status_reaches_the_overview(self):
        fake_bin = self.root / "fake-bin"
        fake_bin.mkdir()
        for provider in ("claude", "codex"):
            executable = fake_bin / provider
            executable.write_text(fake_agent_source())
            executable.chmod(0o755)
        terminal = Terminal(self.root / "agents", general="agent_notifications = false\n")
        terminal.env["PATH"] = f"{fake_bin}{os.pathsep}{terminal.env['PATH']}"
        self.terminals.append(terminal)
        terminal.start()
        terminal.wait_state(lambda s: len(s["tabs"]) == 1, "one tab")
        self.rename(terminal, "Build")
        control = self.root / "build.control"
        control.write_text("working")
        terminal.type_line(f"FAKE_AGENT_CONTROL={control} claude")
        info = wait_for(lambda: read_json(control.with_suffix(".info.json")), "fake claude started")
        self.assertTrue(info["TERMUST_AGENT_RUN"], "the claude proxy did not open a private channel")
        record = Path(info["TERMUST_AGENT_DIR"]) / (info["TERMUST_AGENT_RUN"] + ".json")

        def reach(state):
            control.write_text(state)
            wait_for(lambda: (read_json(record) or {}).get("state") == state, f"agent state {state}")

        reach("working")
        terminal.keystroke("t", "command")
        self.rename(terminal, "Docs")
        terminal.wait_state(lambda s: tab_titles(s) == ["Build", "Docs"] and s["active"] == 1,
            "second tab focused")
        reach("permission")
        time.sleep(1)
        terminal.open_menu()
        for name in ("end", "up", "up", "up", "return", "return"):
            terminal.key(name, settle=0.4)
        terminal.wait_state(lambda s: s["active"] == 0, "overview focused the agent asking permission")
        wait_for(lambda: terminal.osa(" return name of window 1 of p").startswith("Build"),
            "window title of the agent tab")
        control.write_text("exit")

    def test_default_login_shell_records_commands(self):
        terminal = self.terminal("login-shell", isolated_shell=False)
        terminal.wait_state(lambda s: len(s["tabs"]) == 1, "one tab")
        terminal.type_line(f"cd {terminal.root}")
        terminal.type_line("echo termust-login-shell")
        snapshot = terminal.wait_state(lambda s: any("termust-login-shell" in command.get("command", "")
            for command in s["tabs"][0]["commands"]), "recorded login shell command")
        self.assertIsNone(snapshot["tabs"][0].get("shell"))
        self.assertEqual(Path(snapshot["tabs"][0]["working_directory"]).resolve(),
            terminal.root.resolve())


if __name__ == "__main__":
    if platform.system() != "Darwin":
        print("These graphical checks run only on macOS.", file=sys.stderr)
        sys.exit(0)
    unittest.main()
