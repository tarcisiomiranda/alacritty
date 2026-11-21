import argparse
from contextlib import contextmanager
import ctypes
import importlib.util
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import time
import tomllib
import unittest


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


productivity = module("agent_productivity", "test-productivity.py")
groups = module("agent_groups", "test-state-groups.py")
smoke = productivity.smoke
BINARY = None
SCREENSHOT = None


FAKE = r'''#!/usr/bin/env python3
import atexit,json,os,select,subprocess,sys,termios,time,tty
from pathlib import Path
provider=Path(sys.argv[0]).name
if "--version" in sys.argv:
    print(provider+" v2.2.5")
    raise SystemExit(0)
if "--help" in sys.argv:
    print("hooks settings terminal")
    raise SystemExit(0)
control=Path(os.environ["FAKE_AGENT_CONTROL"])
info=control.with_suffix(".info.json")
data={key:os.environ.get(key) for key in ["TERMUST_AGENT_DIR","TERMUST_AGENT_RUN","TERMUST_AGENT_PID","TERMUST_AGENT_BRIDGE","TERMUST_AGENT_PYTHON"]}
data["child_pid"]=os.getpid()
input_path=None
if os.environ.get("FAKE_AGENT_RECORD_INPUT"):
    original=termios.tcgetattr(sys.stdin)
    tty.setraw(sys.stdin)
    atexit.register(termios.tcsetattr,sys.stdin,termios.TCSANOW,original)
    input_path=control.with_suffix(".input.bin")
    input_path.write_bytes(b"")
info.write_text(json.dumps(data))
print("\033[3J\033[2J\033[HLocal agent integration test. No network requests.\n",flush=True)
last=None
while True:
    if input_path and select.select([sys.stdin],[],[],0)[0]:
        with input_path.open("ab") as stream:stream.write(os.read(sys.stdin.fileno(),4096))
    state=control.read_text().strip() if control.exists() else "working"
    if state=="exit":break
    if state!=last and data["TERMUST_AGENT_RUN"]:
        if provider=="agy":
            event="title"
            payload={"agent_state":{"done":"idle","ready":"idle","input":"reviewing","permission":"tool_use"}.get(state,state)}
            if state=="permission":payload["tool_confirmation_pending"]=True
        elif provider=="opencode":event,payload=state,{}
        else:
            event={"ready":"SessionStart","working":"UserPromptSubmit","input":"PreToolUse","permission":"PermissionRequest","done":"Stop","error":"StopFailure"}[state]
            payload={"tool_name":"AskUserQuestion"} if state=="input" else {}
        bridge=data["TERMUST_AGENT_BRIDGE"] or str(Path(data["TERMUST_AGENT_DIR"])/"agent_bridge.py")
        result=subprocess.run([data["TERMUST_AGENT_PYTHON"] or sys.executable,bridge,"hook",provider,event],input=json.dumps(payload),text=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        if result.returncode:raise SystemExit(result.returncode)
        control.with_suffix(".ack").write_text(state)
    last=state
    time.sleep(.04)
'''


@contextmanager
def fixture():
    with tempfile.TemporaryDirectory(prefix="termust-agents-ui-") as directory:
        terminal = productivity.Fixture(BINARY, Path(directory), SCREENSHOT)
        binary_dir = terminal.work / "fake-bin"
        binary_dir.mkdir()
        for provider in ("claude", "codex", "agy", "opencode"):
            executable = binary_dir / provider
            executable.write_text(FAKE)
            executable.chmod(0o755)
        terminal.env["PATH"] = str(binary_dir) + os.pathsep + terminal.env["PATH"]
        terminal.env["WINIT_X11_SCALE_FACTOR"] = "1.0"
        try:
            terminal.start()
            terminal.general(agent_integration=True, agent_notifications=False)
            yield terminal
        except Exception:
            if terminal.app and terminal.app.poll() is None:
                terminal.capture("agents-failure")
            raise
        finally:
            terminal.close()
            terminal.xvfb.stdout.close()


def start_agent(terminal, provider, label, record_input=False):
    control = terminal.work / (label + ".control")
    control.write_text("working")
    input_flag = " FAKE_AGENT_RECORD_INPUT=1" if record_input else ""
    terminal.command("printf '\\033[2J\\033[H'; FAKE_AGENT_CONTROL=" + shlex.quote(str(control)) + input_flag + " " + provider)
    info = control.with_suffix(".info.json")
    smoke.wait_for(info.exists, "isolated " + provider + " started")
    session = json.loads(info.read_text())
    session["control"] = control
    session["provider"] = provider
    if record_input:
        session["input"] = control.with_suffix(".input.bin")
    if session["TERMUST_AGENT_RUN"]:
        session["record"] = Path(session["TERMUST_AGENT_DIR"]) / (session["TERMUST_AGENT_RUN"] + ".json")
        change_state(session, "working")
    return session


def change_state(session, state):
    session["control"].write_text(state)
    ack = session["control"].with_suffix(".ack")
    smoke.wait_for(lambda: ack.exists() and ack.read_text() == state, "agent hook reports " + state)
    record = smoke.wait_for(lambda: read_json(session["record"])
        if read_json(session["record"]) and read_json(session["record"])["state"] == state else None,
        "private channel contains " + state)
    assert set(record) <= {"v", "provider", "state", "run", "pid", "sequence", "updated_ms", "limited"}
    time.sleep(.75)
    return record


def read_json(path):
    try:
        return json.loads(path.read_text())
    except (FileNotFoundError, json.JSONDecodeError):
        return None


def stop_agent(terminal, session, label):
    session["control"].write_text("exit")
    time.sleep(.25)
    return terminal.report(label)


def bar(terminal, label):
    columns, _, _, _ = terminal.dimensions()
    screenshot = terminal.screenshot
    if label not in {"hidden-group-attention", "hidden-group-working"}:
        terminal.screenshot = None
    try:
        return terminal.pixels(terminal.capture(label), 0, -1, columns)
    finally:
        terminal.screenshot = screenshot


def overview(terminal, toolbar=False):
    if toolbar:
        snapshot = terminal.snapshot()
        selected = snapshot.get("active_group")
        count = sum(selected is None or tab.get("group") == selected for tab in snapshot["tabs"])
        label = selected or ("Todas as abas" if terminal.runtime_config()["general"]["language"] == "pt-BR" else "All tabs")
        columns, _, cell_width, cell_height = terminal.dimensions()
        start = min(18, max(8, len(label) + 4), max(0, columns - 12)) if any(tab.get("group") for tab in snapshot["tabs"]) else 0
        available = max(0, columns - start - 9)
        width = min(available, min(30, max(6, available // max(1, count))))
        visible = min(count, available // width) if width else 0
        terminal.x11.click((start + visible * width + 7) * cell_width, cell_height / 2)
    else:
        terminal.menu()
        terminal.x11.chord("End")
        for _ in range(3):
            terminal.x11.chord("Up")
        terminal.x11.chord("Return")
    time.sleep(.2)


def chat_panel_pixels(terminal, path):
    width, height = terminal.x11.geometry(terminal.window)
    left, top = width // 2, int(terminal.dimensions()[3])
    crop_width = width - left
    crop = f"{crop_width}x{height // 3}+{left}+{top}"
    pixels = subprocess.run(
        ["convert", str(path), "-crop", crop, "+repage", "-depth", "8", "rgb:-"],
        capture_output=True, check=True, timeout=5,
    ).stdout
    return pixels, (left, top, crop_width)


def new_tab(terminal, title):
    terminal.x11.chord("Control_L", "Shift_L", "t")
    terminal.rename(title)
    return terminal.report(title + "-shell")[0]


class AgentTests(unittest.TestCase):
    def test_installed_agy_title_callback_inherits_private_channel(self):
        if shutil.which("agy") is None:
            self.skipTest("Antigravity CLI is not installed")
        with tempfile.TemporaryDirectory(prefix="termust-agy-real-") as directory:
            terminal = productivity.Fixture(BINARY, Path(directory), None)
            try:
                terminal.start()
                root_path = terminal.work / "private-root"
                terminal.command("printf '%s' \"$TERMUST_AGENT_DIR\" > " + shlex.quote(str(root_path)))
                smoke.wait_for(root_path.exists, "native pane integration channel available")
                root = Path(root_path.read_text())
                self.assertTrue(root.is_absolute())
                terminal.command("agy")
                def connected():
                    return next((record for path in root.glob("*.json")
                        if (record := read_json(path)) and record.get("provider") == "agy"
                        and record.get("state") in {"ready", "working", "input"}
                        and record.get("limited") is False), None)
                record = smoke.wait_for(connected, "installed Antigravity emitted its structured title state")
                self.assertEqual(record["v"], 1)
                terminal.x11.chord("Control_L", "d")
            finally:
                terminal.close()
                terminal.xvfb.stdout.close()

    def test_installed_opencode_v2_loads_private_tui_plugin(self):
        executable = shutil.which("opencode")
        if executable is None:
            self.skipTest("OpenCode is not installed")
        version = subprocess.check_output([executable, "--version"], text=True, timeout=5)
        if "v2." not in version:
            self.skipTest("native smoke targets the installed OpenCode 2 interface")
        with tempfile.TemporaryDirectory(prefix="termust-opencode-real-") as directory:
            terminal = productivity.Fixture(BINARY, Path(directory), None)
            try:
                terminal.start()
                root_path = terminal.work / "private-root"
                terminal.command("printf '%s' \"$TERMUST_AGENT_DIR\" > " + shlex.quote(str(root_path)))
                smoke.wait_for(root_path.exists, "native pane integration channel available")
                root = Path(root_path.read_text())
                self.assertTrue(root.is_absolute())
                self.assertTrue(root.is_dir())
                terminal.command("opencode --standalone " + shlex.quote(str(terminal.work)))
                def connected():
                    return next((record for path in root.glob("*.json")
                        if (record := read_json(path)) and record.get("provider") == "opencode"
                        and record.get("state") == "ready" and record.get("limited") is False), None)
                record = smoke.wait_for(connected, "installed OpenCode emitted its TUI ready event")
                self.assertEqual(record["v"], 1)
                terminal.x11.chord("Control_L", "c")
                terminal.x11.chord("Control_L", "c")
            finally:
                terminal.close()
                terminal.xvfb.stdout.close()

    def test_all_provider_states_are_painted_without_renaming_tabs(self):
        with fixture() as terminal:
            for index, provider in enumerate(("claude", "codex", "agy", "opencode")):
                if index:
                    new_tab(terminal, provider.title())
                else:
                    terminal.rename(provider.title())
                shell = terminal.report(provider + "-original")[0]
                session = start_agent(terminal, provider, provider)
                self.assertIsNotNone(session["TERMUST_AGENT_RUN"])
                previous = bar(terminal, provider + "-working")
                for state in ("input", "permission", "done", "error"):
                    change_state(session, state)
                    painted = bar(terminal, provider + "-" + state)
                    self.assertNotEqual(previous, painted, provider + " state did not repaint tab")
                    self.assertEqual(terminal.x11.title(terminal.window), provider.title())
                    previous = painted
                self.assertEqual(stop_agent(terminal, session, provider + "-after")[0], shell)
            self.assertEqual(len(terminal.snapshot()["tabs"]), 4)

    def test_hidden_group_attention_focuses_the_correct_split_pane(self):
        with fixture() as terminal:
            terminal.rename("Build")
            groups.group_action(terminal, "create", name="Project")
            left = terminal.report("left-shell")[0]
            terminal.command("printf '\\033[2J\\033[HWorkspace ready\\n'")
            terminal.x11.chord("Control_L", "Shift_L", "e")
            right = terminal.report("right-shell")[0]
            build = start_agent(terminal, "codex", "build")
            docs_shell = new_tab(terminal, "Docs")
            groups.group_action(terminal, "create", name="Research")
            docs = start_agent(terminal, "opencode", "docs")
            before = bar(terminal, "hidden-group-working")
            change_state(build, "permission")
            after = bar(terminal, "hidden-group-attention")
            self.assertNotEqual(before, after, "hidden group did not update global attention badge")
            overview(terminal, toolbar=True)
            terminal.x11.chord("Return")
            smoke.wait_for(lambda: terminal.x11.title(terminal.window) == "Build", "overview focuses hidden grouped tab")
            snapshot = terminal.snapshot()
            self.assertEqual(snapshot["active_group"], "Project")
            self.assertEqual(snapshot["tabs"][snapshot["active"]]["active_pane"], 1)
            focused = stop_agent(terminal, build, "focused-right-shell")[0]
            self.assertEqual(focused, right)
            self.assertNotEqual(focused, left)
            change_state(docs, "input")
            overview(terminal)
            terminal.x11.chord("Return")
            smoke.wait_for(lambda: terminal.x11.title(terminal.window) == "Docs", "overview omits exited agent and focuses remaining live tab")
            self.assertEqual(stop_agent(terminal, docs, "focused-docs-shell")[0], docs_shell)

    def test_focusing_permission_pane_does_not_forward_enter_or_key_repeat(self):
        with fixture() as terminal:
            terminal.rename("Permission")
            left = terminal.report("permission-left-shell")[0]
            terminal.x11.chord("Control_L", "Shift_L", "e")
            right = terminal.report("permission-right-shell")[0]
            agent = start_agent(terminal, "codex", "raw-permission", record_input=True)
            new_tab(terminal, "Notes")
            change_state(agent, "permission")
            overview(terminal)
            self.assertEqual(agent["input"].read_bytes(), b"", "opening the overview sent input to the agent")
            enter = terminal.x11.keycode("Return")
            terminal.x11.press(enter, True)
            terminal.x11.lib.XFlush(terminal.x11.display)
            time.sleep(.9)
            terminal.x11.press(enter, False)
            terminal.x11.lib.XFlush(terminal.x11.display)
            smoke.wait_for(lambda: terminal.x11.title(terminal.window) == "Permission", "overview focuses permission pane")
            time.sleep(.2)
            self.assertEqual(agent["input"].read_bytes(), b"", "overview selection forwarded Enter or a repeat to the permission input")
            terminal.x11.type("n")
            smoke.wait_for(lambda: agent["input"].read_bytes() == b"n", "explicit user key reaches the focused pane")
            self.assertEqual(agent["input"].read_bytes(), b"n")
            focused = stop_agent(terminal, agent, "raw-permission-focused-shell")[0]
            self.assertEqual(focused, right)
            self.assertNotEqual(focused, left)

    def test_compact_chat_list_animates_and_mouse_focuses_waiting_split(self):
        with fixture() as terminal:
            terminal.rename("Parallel")
            left = start_agent(terminal, "claude", "parallel-left")
            terminal.x11.chord("Control_L", "Shift_L", "e")
            right_shell = terminal.report("parallel-right-shell")[0]
            right = start_agent(terminal, "codex", "parallel-right", record_input=True)
            overview(terminal, toolbar=True)

            def frames(label):
                return {chat_panel_pixels(terminal, terminal.capture(label))[0] for _ in range(4)}

            self.assertGreater(len(frames("chats-working")), 1,
                "working animation stopped without new agent events")
            change_state(left, "ready")
            change_state(right, "permission")
            self.assertGreater(len(frames("chats-permission")), 1,
                "permission indicator did not pulse without new agent events")
            self.assertEqual(right["input"].read_bytes(), b"")
            pixels, (left_edge, top, width) = chat_panel_pixels(
                terminal, terminal.capture("chats-compact-waiting"))
            permission_pixels = []
            for offset in range(0, len(pixels), 3):
                red, green, blue = pixels[offset:offset + 3]
                if red > green + 8 and green > blue + 8:
                    permission_pixels.append(offset // 3)
            self.assertTrue(permission_pixels, "the yellow permission status was not rendered")
            row, column = divmod(permission_pixels[len(permission_pixels) // 2], width)
            terminal.x11.click(left_edge + column, top + row)
            smoke.wait_for(lambda: terminal.snapshot()["tabs"][0]["active_pane"] == 1,
                "status click focuses the waiting split")
            self.assertEqual(right["input"].read_bytes(), b"", "click forwarded input to the permission")
            terminal.x11.type("n")
            smoke.wait_for(lambda: right["input"].read_bytes() == b"n",
                "explicit input reaches the selected chat")
            change_state(right, "ready")
            self.assertEqual(bar(terminal, "chats-ready"), bar(terminal, "chats-still"),
                "idle chats keep animating")
            self.assertEqual(stop_agent(terminal, right, "parallel-focused-shell")[0], right_shell)
            left["control"].write_text("exit")

    def test_tab_indicator_style_preserves_titles_and_applies_without_restarting(self):
        with fixture() as terminal:
            terminal.rename("Shell")
            title = "Minecraft build review"
            new_tab(terminal, title)
            columns, _, _, _ = terminal.dimensions()
            tab_width = min(30, (columns - 9) // 2)

            def sample(label):
                path = terminal.capture(label)
                icon = terminal.pixels(path, tab_width + 1, -1, 1)
                name = terminal.pixels(path, tab_width + 3, -1, tab_width - 7)
                return icon, name

            def choose_animation(enabled):
                terminal.preferences("agents")
                terminal.tabs(4 if enabled else 5)
                terminal.x11.chord("space")
                terminal.capture("indicator-settings")
                terminal.x11.chord("Control_L", "Return")
                smoke.wait_for(
                    lambda: terminal.runtime_config()["general"]["agent_status_animation"] is enabled,
                    "status indicator preference applied",
                )
                terminal.x11.chord("Escape")
                saved = tomllib.loads(terminal.config.read_text())
                self.assertIs(saved["general"]["agent_status_animation"], enabled)

            _, original_title = sample("chat-title-before-agent")
            agent = start_agent(terminal, "codex", "named-chat")
            animated = [sample("chat-title-animated") for _ in range(4)]
            self.assertTrue(all(name == original_title for _, name in animated),
                "the working state reduced the rendered title")
            self.assertGreater(len({icon for icon, _ in animated}), 1)

            choose_animation(False)
            static = [sample("chat-title-static") for _ in range(4)]
            self.assertEqual(len({icon for icon, _ in static}), 1)
            self.assertTrue(all(name == original_title for _, name in static),
                "static icon mode changed the rendered title")
            overview(terminal, toolbar=True)
            popup = {
                chat_panel_pixels(terminal, terminal.capture("static-chat-status"))[0]
                for _ in range(4)
            }
            self.assertEqual(len(popup), 1, "chat list animates in static icon mode")
            terminal.x11.chord("Escape")

            choose_animation(True)
            resumed = [sample("chat-title-animation-resumed") for _ in range(4)]
            self.assertGreater(len({icon for icon, _ in resumed}), 1)
            change_state(agent, "done")
            finished = sample("chat-title-finished")
            self.assertEqual(finished, sample("chat-title-finished"))
            self.assertNotEqual(finished[0], static[0][0])
            self.assertEqual(finished[1], original_title)
            self.assertEqual(terminal.x11.title(terminal.window), title)
            agent["control"].write_text("exit")

    def test_unsafe_records_are_ignored_and_native_setting_disables_new_shells(self):
        with fixture() as terminal:
            terminal.rename("Integrity")
            session = start_agent(terminal, "claude", "integrity")
            change_state(session, "ready")
            original = read_json(session["record"])
            painted = bar(terminal, "valid-agent-record")
            for invalid in ({**original, "run": "invalid", "state": "permission"},
                    {**original, "sequence": 0, "state": "permission"}):
                session["record"].write_text(json.dumps(invalid))
                time.sleep(.8)
                self.assertEqual(bar(terminal, "rejected-agent-record"), painted)
            session["record"].write_text("x" * 5000)
            time.sleep(.8)
            self.assertEqual(bar(terminal, "oversized-agent-record"), painted)
            session["record"].write_text(json.dumps(original))
            terminal.preferences("agents")
            terminal.tabs(2)
            terminal.x11.chord("space")
            terminal.x11.chord("Control_L", "Return")
            smoke.wait_for(lambda: terminal.runtime_config()["general"]["agent_integration"] is False,
                "native integration preference saved")
            terminal.x11.chord("Escape")
            new_tab(terminal, "Plain")
            plain = start_agent(terminal, "claude", "plain")
            self.assertFalse(plain["TERMUST_AGENT_RUN"])
            stop_agent(terminal, plain, "ordinary-shell-after-disabled-agent")
            marker = terminal.work / "ordinary-command"
            terminal.command("printf normal > " + shlex.quote(str(marker)))
            smoke.wait_for(marker.exists, "ordinary commands remain usable")
            self.assertEqual(marker.read_text(), "normal")
            session["control"].write_text("exit")

    def test_localized_overview_is_readable_and_mouse_focus_is_stable(self):
        with fixture() as terminal:
            terminal.rename("Editor")
            editor = start_agent(terminal, "claude", "editor")
            new_tab(terminal, "Tests")
            tests = start_agent(terminal, "agy", "tests")
            change_state(editor, "done")
            change_state(tests, "done")
            terminal.x11.lib.XResizeWindow(terminal.x11.display, terminal.window, 1176, 790)
            terminal.x11.lib.XFlush(terminal.x11.display)
            time.sleep(.3)
            overview(terminal)

            english, _ = chat_panel_pixels(terminal, terminal.capture("agents-overview-en"))
            terminal.general(language="pt-BR")
            localized, _ = chat_panel_pixels(terminal, terminal.capture("agents-live-language"))
            terminal.x11.chord("Escape")
            overview(terminal)
            reopened, _ = chat_panel_pixels(terminal, terminal.capture("agents-reopened-language"))
            self.assertNotEqual(english, localized, "changing language left the open dialog unchanged")
            self.assertEqual(localized, reopened, "live dialog statuses differ from a newly opened localized dialog")
            change_state(editor, "permission")
            change_state(tests, "input")
            terminal.x11.lib.XResizeWindow(terminal.x11.display, terminal.window, 737, 570)
            terminal.x11.lib.XFlush(terminal.x11.display)
            time.sleep(.3)
            terminal.capture("agents-overview-pt-BR")
            terminal.x11.chord("Escape")
            terminal.x11.chord("Control_L", "Shift_L", "t")
            terminal.refresh_size()
            terminal.rename("Notes")
            overview(terminal)
            terminal.x11.chord("Return")
            smoke.wait_for(lambda: terminal.x11.title(terminal.window) == "Editor", "localized overview keeps target identity")
            editor["control"].write_text("exit")
            tests["control"].write_text("exit")

    @unittest.skipUnless(shutil.which("xprop"), "xprop is needed to inspect X11 attention")
    def test_window_attention_respects_notification_preference(self):
        with fixture() as terminal:
            session = start_agent(terminal, "claude", "notifications")
            terminal.general(agent_notifications=True)
            root_window = terminal.x11.lib.XDefaultRootWindow
            root_window.argtypes = [ctypes.c_void_p]
            root_window.restype = ctypes.c_ulong
            def focus(window):
                terminal.x11.lib.XSetInputFocus(terminal.x11.display, window, 2, 0)
                terminal.x11.lib.XFlush(terminal.x11.display)
                time.sleep(.25)
            def urgent():
                return "urgency" in subprocess.check_output(["xprop", "-display", terminal.display,
                    "-id", str(terminal.window), "WM_HINTS"], text=True).lower()
            focus(root_window(terminal.x11.display))
            change_state(session, "permission")
            smoke.wait_for(urgent, "background approval requests window attention")
            focus(terminal.window)
            smoke.wait_for(lambda: not urgent(), "focusing the window clears attention")
            terminal.general(agent_notifications=False)
            change_state(session, "working")
            focus(root_window(terminal.x11.display))
            change_state(session, "input")
            self.assertFalse(urgent(), "disabled notifications still requested window attention")
            focus(terminal.window)
            stop_agent(terminal, session, "notification-shell-survived")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--screenshot", type=Path)
    parser.add_argument("--case")
    args = parser.parse_args()
    if args.binary:
        BINARY = str(args.binary.resolve())
    else:
        metadata = json.loads(subprocess.check_output([
            "cargo", "metadata", "--no-deps", "--format-version", "1"], text=True))
        BINARY = str(Path(metadata["target_directory"]) / "debug/termust")
    SCREENSHOT = args.screenshot
    suite = unittest.defaultTestLoader.loadTestsFromName(args.case, AgentTests) if args.case else unittest.defaultTestLoader.loadTestsFromTestCase(AgentTests)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    raise SystemExit(not result.wasSuccessful())
