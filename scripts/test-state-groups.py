import argparse
from contextlib import contextmanager
import importlib.util
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import time
import unittest


module = importlib.util.spec_from_file_location(
    "state_groups_productivity", Path(__file__).with_name("test-productivity.py"))
productivity = importlib.util.module_from_spec(module)
module.loader.exec_module(productivity)
smoke = productivity.smoke
BINARY = None
SCREENSHOT = None


@contextmanager
def fixture():
    with tempfile.TemporaryDirectory(prefix="termust-state-groups-") as directory:
        terminal = productivity.Fixture(BINARY, Path(directory), SCREENSHOT)
        try:
            terminal.env["WINIT_X11_SCALE_FACTOR"] = "1.0"
            terminal.start()
            yield terminal
        except Exception:
            terminal.log.flush()
            print((terminal.work / "app.log").read_text()[-5000:], flush=True)
            if terminal.app and terminal.app.poll() is None:
                terminal.capture("state-groups-failure")
            raise
        finally:
            terminal.close()
            terminal.xvfb.stdout.close()


def state(terminal):
    return terminal.snapshot()


def assert_active(terminal, title):
    smoke.wait_for(lambda: terminal.x11.title(terminal.window) == title,
        "active grouped tab is " + title)


def select(terminal, index, title):
    terminal.x11.chord("Alt_L", str(index + 1))
    assert_active(terminal, title)


def new_tab(terminal, title):
    terminal.x11.chord("Control_L", "Shift_L", "t")
    terminal.rename(title)
    return terminal.report(title + "-pid")[0]


def open_groups(terminal, chip=False):
    if chip:
        _, _, cell_width, cell_height = terminal.dimensions()
        terminal.x11.click(cell_width * 1.5, cell_height / 2)
    else:
        terminal.menu()
        terminal.x11.chord("End")
        terminal.x11.chord("Up")
        terminal.x11.chord("Up")
        terminal.x11.chord("Return")
    time.sleep(.1)


def group_action(terminal, action, name=None, selected=None, chip=False, mouse=False):
    snapshot = state(terminal)
    groups = sorted({tab["group"] for tab in snapshot["tabs"] if tab.get("group")})
    current = snapshot["tabs"][snapshot["active"]].get("group")
    open_groups(terminal, chip)
    controls = [0, 1] + ([2] if groups else []) + list(range(100, 100 + len(groups)))
    controls += ([3, 4] if groups else []) + ([5] if current else []) + [6, 7]
    position = 0
    if selected is not None:
        target = controls.index(100 + groups.index(selected))
        terminal.tabs(target)
        terminal.x11.chord("Return")
        position = target
    if name is not None:
        terminal.tabs(position, reverse=True)
        position = 0
        terminal.x11.chord("Control_L", "a")
        terminal.x11.type(name)
    target = controls.index({"create": 1, "rename": 2, "move": 3,
        "show": 4, "ungroup": 5, "all": 6, "cancel": 7}[action])
    if mouse:
        columns, lines, _, _ = terminal.dimensions()
        control_height = 2 if lines >= 26 else 1
        width = min(104, columns - (4 if columns >= 12 else 0))
        desired = 9 + (5 + len(groups)) * control_height + (not groups)
        height = min(desired, lines - (2 if lines >= 22 else 0))
        left, top = (columns - width) // 2 + 2, (lines - height) // 2
        if action == "create":
            body_width = width - 6
            portuguese = terminal.runtime_config()["general"]["language"] == "pt-BR"
            labels = ("Criar grupo", "Renomear grupo") if portuguese else ("Create group", "Rename group")
            available = body_width - 2
            create_width = min(max(len(labels[0]) + 4, 10), available // 2)
            rename_width = min(max(len(labels[1]) + 4, 10), available - create_width)
            start = left + body_width - create_width - rename_width - 2
            terminal.click(start + create_width // 2, top + 5 + control_height)
        elif action == "move":
            terminal.click(left + 3, top + height - (3 * control_height + 3))
        else:
            raise ValueError(action)
    else:
        terminal.tabs(abs(target - position), reverse=target < position)
        terminal.x11.chord("Return")
    time.sleep(.15)
    return state(terminal)


def resize(terminal, width, height):
    terminal.x11.lib.XResizeWindow(terminal.x11.display, terminal.window, width, height)
    terminal.x11.lib.XFlush(terminal.x11.display)
    time.sleep(.2)
    terminal.refresh_size()


def read_json(path):
    if not path.exists():
        return None
    try:
        return json.loads(path.read_text())
    except json.JSONDecodeError:
        return None


class StateGroupsTests(unittest.TestCase):
    def test_groups_create_move_rename_filter_and_reopen_preserve_shells(self):
        with fixture() as terminal:
            terminal.rename("Alpha")
            alpha = terminal.report("alpha-pid")[0]
            snapshot = group_action(terminal, "create", name="Work")
            self.assertEqual(snapshot["active_group"], "Work")
            beta = new_tab(terminal, "Beta")
            gamma = new_tab(terminal, "Gamma")
            snapshot = group_action(terminal, "create", name="Home", chip=True)
            self.assertEqual(snapshot["active_group"], "Home")
            self.assertEqual([tab.get("group") for tab in snapshot["tabs"]], ["Work", "Work", "Home"])
            snapshot = group_action(terminal, "all", chip=True)
            self.assertIsNone(snapshot.get("active_group"))
            delta = new_tab(terminal, "Delta")
            snapshot = group_action(terminal, "move", selected="Work", mouse=True)
            self.assertEqual(snapshot["tabs"][3]["group"], "Work")
            select(terminal, 0, "Alpha")
            group_action(terminal, "move", selected="Home")
            snapshot = group_action(terminal, "show", selected="Work")
            self.assertEqual(snapshot["active_group"], "Work")
            assert_active(terminal, "Beta")
            select(terminal, 1, "Delta")
            self.assertEqual(terminal.report("filtered-delta")[0], delta)
            terminal.x11.chord("Control_L", "Page_Down")
            assert_active(terminal, "Beta")
            terminal.x11.chord("Control_L", "Page_Up")
            assert_active(terminal, "Delta")
            self.assertEqual(state(terminal)["active"], 3)
            terminal.x11.chord("Control_L", "Shift_L", "w")
            assert_active(terminal, "Beta")
            self.assertEqual(len(state(terminal)["tabs"]), 3)
            terminal.x11.chord("Control_L", "Shift_L", "Alt_L", "t")
            assert_active(terminal, "Delta")
            snapshot = state(terminal)
            self.assertEqual(snapshot["tabs"][3]["group"], "Work")
            self.assertEqual(snapshot["active_group"], "Work")
            self.assertNotEqual(terminal.report("reopened-delta")[0], delta)
            snapshot = group_action(terminal, "rename", name="Development", chip=True)
            self.assertEqual(snapshot["active_group"], "Development")
            self.assertEqual([tab.get("group") for tab in snapshot["tabs"]],
                ["Home", "Development", "Home", "Development"])
            group_action(terminal, "all", chip=True)
            select(terminal, 0, "Alpha")
            snapshot = group_action(terminal, "ungroup")
            self.assertIsNone(snapshot["tabs"][0].get("group"))
            self.assertEqual(terminal.report("alpha-still-running")[0], alpha)
            select(terminal, 1, "Beta")
            self.assertEqual(terminal.report("beta-still-running")[0], beta)
            select(terminal, 2, "Gamma")
            self.assertEqual(terminal.report("gamma-still-running")[0], gamma)
            for pid in (alpha, beta, gamma):
                os.kill(pid, 0)
            group_action(terminal, "show", selected="Home", chip=True)
            assert_active(terminal, "Gamma")
            terminal.x11.chord("Control_L", "Shift_L", "w")
            snapshot = state(terminal)
            self.assertEqual(len(snapshot["tabs"]), 3)
            self.assertIsNone(snapshot.get("active_group"))
            self.assertNotIn("Gamma", [tab.get("title") for tab in snapshot["tabs"]])

    def test_manual_sessions_and_startup_workspaces_keep_groups_panes_and_output(self):
        with fixture() as terminal:
            terminal.rename("Editor")
            group_action(terminal, "create", name="Project")
            terminal.command("printf '\\033[2J\\033[HEDITOR-CONTEXT-729\\n'")
            new_tab(terminal, "Services")
            group_action(terminal, "create", name="Servers")
            terminal.command("printf 'SERVER-PRIMARY-892\\n'")
            terminal.x11.chord("Control_L", "Shift_L", "e")
            terminal.command("printf 'SERVER-SPLIT-568\\n'")
            terminal.report("split-ready")
            snapshot = state(terminal)
            self.assertEqual(snapshot["active_group"], "Servers")
            self.assertEqual(len(snapshot["tabs"][1]["panes"]), 1)
            self.assertIn("EDITOR-CONTEXT-729", snapshot["tabs"][0]["screen_text"])
            self.assertIn("SERVER-PRIMARY-892", snapshot["tabs"][1]["screen_text"])
            self.assertIn("SERVER-SPLIT-568", snapshot["tabs"][1]["panes"][0]["screen_text"])
            terminal.save_workspace("Daily")
            workspace = read_json(terminal.state_directory / "workspaces.json")["workspaces"]["Daily"]
            self.assertEqual(workspace["active_group"], "Servers")
            self.assertEqual([tab["group"] for tab in workspace["tabs"]], ["Project", "Servers"])
            terminal.stop(crash=True)
            terminal.start()
            assert_active(terminal, "Services")
            restored = state(terminal)
            self.assertEqual(restored["active_group"], "Servers")
            self.assertEqual([tab["group"] for tab in restored["tabs"]], ["Project", "Servers"])
            self.assertEqual(len(restored["tabs"][1]["panes"]), 1)
            self.assertIn("EDITOR-CONTEXT-729", restored["tabs"][0]["screen_text"])
            self.assertIn("SERVER-SPLIT-568", restored["tabs"][1]["panes"][0]["screen_text"])
            group_action(terminal, "rename", name="Temporary", chip=True)
            terminal.general(startup_session="Daily")
            terminal.stop(crash=True)
            terminal.start()
            restored = state(terminal)
            self.assertEqual(restored["active_group"], "Servers")
            self.assertEqual([tab["group"] for tab in restored["tabs"]], ["Project", "Servers"])
            self.assertEqual(len(restored["tabs"][1]["panes"]), 1)
            self.assertIn("SERVER-PRIMARY-892", restored["tabs"][1]["screen_text"])

    def test_recovery_keeps_running_command_and_recent_text_without_reexecution(self):
        with fixture() as terminal:
            terminal.rename("Manual")
            state(terminal)
            manual_path = terminal.state_directory / "tabs.json"
            manual = manual_path.read_bytes()
            terminal.general(session_autosave=True)
            terminal.rename("Recovery")
            group_action(terminal, "create", name="Build")
            terminal.toggle_replay()
            manual_path.write_bytes(manual)
            count = terminal.work / "execution-count"
            gate = terminal.work / "finish-command"
            command = (f"printf 'once\\n' >> {shlex.quote(str(count))}; "
                "printf '\\033[2J\\033[HRECOVERY-OUTPUT-483\\n'; "
                f"while test ! -e {shlex.quote(str(gate))}; do sleep 0.05; done")
            terminal.command(command)
            smoke.wait_for(count.exists, "long command started exactly once")
            recovery_path = terminal.state_directory / "recovery.json"
            def recovered_command():
                saved = read_json(recovery_path)
                if not saved or saved["tabs"][0].get("title") != "Recovery":
                    return None
                commands = saved["tabs"][0].get("commands", [])
                if not commands or commands[-1]["command"] != command:
                    return None
                return saved if "RECOVERY-OUTPUT-483" in saved["tabs"][0].get("screen_text", "") else None
            saved = smoke.wait_for(recovered_command,
                "automatic recovery captured output and the still-running last command")
            self.assertEqual(saved["tabs"][0]["commands"][-1]["working_directory"], str(terminal.work))
            self.assertEqual(saved["active_group"], "Build")
            self.assertTrue(saved["tabs"][0]["replay_last_command"])
            self.assertEqual(manual_path.read_bytes(), manual)
            self.assertEqual(recovery_path.stat().st_mode & 0o777, 0o600)
            self.assertEqual(count.read_text().splitlines(), ["once"])
            terminal.stop(crash=True)
            terminal.start()
            assert_active(terminal, "Recovery")
            restored = state(terminal)
            self.assertEqual(restored["active_group"], "Build")
            self.assertIn(command, [record["command"] for record in restored["tabs"][0]["commands"]])
            self.assertIn("RECOVERY-OUTPUT-483", restored["tabs"][0]["screen_text"])
            self.assertEqual(count.read_text().splitlines(), ["once"])
            self.assertEqual(terminal.report("restored-directory")[1], str(terminal.work))

    def test_group_dialog_mouse_keyboard_cancel_and_localized_compact_layout(self):
        with fixture() as terminal:
            terminal.rename("Editor")
            pid = terminal.report("ui-original")[0]
            group_action(terminal, "create", name="Project", mouse=True)
            new_tab(terminal, "API")
            group_action(terminal, "create", name="Servers")
            group_action(terminal, "all", chip=True)
            for locale, width, height in (("en", 1176, 790), ("pt-BR", 737, 570)):
                terminal.general(language=locale)
                resize(terminal, width, height)
                terminal.command("printf '\\033[2J\\033[H'")
                time.sleep(.1)
                open_groups(terminal, chip=True)
                terminal.capture("groups-" + locale + "-" + str(width))
                terminal.x11.chord("Control_L", "a")
                terminal.x11.type("Discarded")
                terminal.x11.chord("Escape")
                snapshot = state(terminal)
                self.assertEqual([tab["group"] for tab in snapshot["tabs"]], ["Project", "Servers"])
                self.assertIsNone(snapshot.get("active_group"))
                terminal.capture("groups-bar-" + locale + "-" + str(width))
                group_action(terminal, "show", selected="Servers", chip=True)
                terminal.capture("groups-filtered-" + locale + "-" + str(width))
                group_action(terminal, "all", chip=True)
            select(terminal, 0, "Editor")
            self.assertEqual(terminal.report("ui-still-running")[0], pid)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--screenshot", type=Path)
    parser.add_argument("--case", action="append", choices=[name for name in dir(StateGroupsTests)
        if name.startswith("test_")])
    args = parser.parse_args()
    if args.binary:
        BINARY = str(args.binary.resolve())
    else:
        metadata = json.loads(subprocess.check_output([
            "cargo", "metadata", "--no-deps", "--format-version", "1"], text=True))
        BINARY = str(Path(metadata["target_directory"]) / "debug/termust")
    SCREENSHOT = args.screenshot
    suite = unittest.TestSuite(StateGroupsTests(name) for name in args.case) if args.case else (
        unittest.defaultTestLoader.loadTestsFromTestCase(StateGroupsTests))
    raise SystemExit(not unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful())
