import argparse
from contextlib import contextmanager
import importlib.util
import json
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import time
import tomllib
import unittest


def load_fixture(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


panes = load_fixture("test-panes")
confirmation = load_fixture("test-close-confirmation")
productivity = panes.productivity
language = productivity.language
smoke = language.smoke
BINARY = None


@contextmanager
def isolated_fixture(kind=productivity.Fixture):
    with tempfile.TemporaryDirectory(prefix="alacritty-regression-") as directory:
        fixture = kind(BINARY, Path(directory), None)
        try:
            yield fixture
        except Exception:
            fixture.log.flush()
            print((fixture.work / "app.log").read_text()[-3000:], flush=True)
            raise
        finally:
            fixture.close()


def focus_window(fixture, window):
    fixture.x11.lib.XSetInputFocus(fixture.x11.display, window, 2, 0)
    fixture.x11.lib.XFlush(fixture.x11.display)


class BugRegressions(unittest.TestCase):
    def check_replay(self, source):
        with isolated_fixture(language.Fixture) as fixture:
            work = fixture.work
            gate = work / "ready"
            loaded = work / "loaded"
            replayed = work / "saved-command"
            typed = work / "typed-command"
            fixture.config.write_text(fixture.config.read_text().replace(
                '["--noprofile", "--norc"]', '["--noprofile"]'))
            (work / "home/.bashrc").write_text(
                f"while [[ ! -e {shlex.quote(str(gate))} ]]; do sleep .05; done\n"
                f"touch {shlex.quote(str(loaded))}\n")
            snapshot = {
                "version": 1, "active": 0,
                "tabs": [{
                    "title": None, "working_directory": str(work),
                    "commands": [{"command": f"touch {shlex.quote(str(replayed))}",
                                  "working_directory": str(work)}],
                    "replay_last_command": True,
                    "shell": {"program": "/bin/bash", "args": ["--noprofile"]},
                }],
            }
            state = work / "state/termust"
            state.mkdir()
            (state / "tabs.json").write_text(json.dumps(snapshot))
            fixture.app = subprocess.Popen([
                fixture.binary, "--config-file", str(fixture.config),
                "--socket", str(fixture.socket)], env=fixture.env,
                stdout=fixture.log, stderr=fixture.log)
            fixture.window = smoke.wait_for(lambda: next((window
                for window in fixture.x11.windows()
                if fixture.x11.title(window) == "language-ui"), None),
                "delayed shell window opened")
            time.sleep(0.4)
            focus_window(fixture, fixture.window)
            clipboard = None
            try:
                command = f"touch {shlex.quote(str(typed))} #"
                if source == "keyboard":
                    fixture.x11.type(command)
                elif source == "clipboard":
                    clipboard = subprocess.Popen([
                        "xclip", "-selection", "clipboard", "-in", "-quiet"],
                        env=fixture.env, stdin=subprocess.PIPE,
                        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                    clipboard.stdin.write(command.encode())
                    clipboard.stdin.close()
                    time.sleep(0.15)
                    fixture.x11.chord("Control_L", "Shift_L", "v")
                gate.touch()
                smoke.wait_for(loaded.exists, "delayed shell startup completed")
                if source is None:
                    smoke.wait_for(replayed.exists, "untouched saved command replayed")
                else:
                    time.sleep(1)
                    self.assertFalse(typed.exists(), "input ran without the user pressing Enter")
                    self.assertFalse(replayed.exists(), "user input did not cancel pending replay")
                    fixture.x11.chord("Return")
                    smoke.wait_for(typed.exists, "user input runs only after explicit Enter")
                    self.assertFalse(replayed.exists(), "canceled replay ran after user command")
            finally:
                if clipboard and clipboard.poll() is None:
                    clipboard.terminate()
                    clipboard.wait(timeout=5)

    def test_replay_canceled_by_typing(self):
        self.check_replay("keyboard")

    @unittest.skipUnless(shutil.which("xclip"), "xclip is required for the clipboard regression")
    def test_replay_canceled_by_paste(self):
        self.check_replay("clipboard")

    def test_replay_without_input_still_runs(self):
        self.check_replay(None)

    def test_modal_enter_repeats_stay_out_of_shell(self):
        for dialog in ("rename", "settings-save", "settings-cancel", "workspaces-cancel"):
            with self.subTest(dialog=dialog), isolated_fixture(confirmation.Fixture) as fixture:
                fixture.start()
                marker = fixture.work / "executed"
                fixture.x11.type("touch " + shlex.quote(str(marker)))
                if dialog == "rename":
                    fixture.x11.chord("Control_L", "Shift_L", "r")
                    fixture.x11.chord("Control_L", "a")
                    fixture.x11.type("Renamed")
                elif dialog.startswith("settings"):
                    fixture.preferences()
                    fixture.tabs(2 if dialog == "settings-save" else 1, reverse=True)
                else:
                    fixture.x11.chord("Control_L", "Shift_L", "p")
                    fixture.tabs(1, reverse=True)
                fixture.hold("Return")
                self.assertFalse(marker.exists(), f"{dialog} leaked Enter repeats to the shell")
                if dialog == "settings-save":
                    fixture.x11.chord("Escape")
                fixture.x11.chord("Return")
                smoke.wait_for(marker.exists, f"{dialog} allows a new Enter after release")

    def test_settings_rebase_external_changes(self):
        for edit_font in (False, True):
            with self.subTest(edited_font=edit_font), isolated_fixture() as fixture:
                fixture.start()
                baseline = fixture.runtime_config()["font"]["size"]
                fixture.preferences()
                if edit_font:
                    fixture.tabs(8)
                    fixture.x11.chord("Return")
                    smoke.wait_for(lambda: fixture.runtime_config()["font"]["size"] == baseline + 0.5,
                        "font draft edited before external reload")
                fixture.config.write_text(fixture.config.read_text().replace(
                    "confirm_tab_close = false", "confirm_tab_close = true")
                    + '\n[font]\nsize = 18.0\n[font.normal]\nfamily = "DejaVu Sans Mono"\n')
                smoke.wait_for(lambda: language.Fixture.runtime_config(fixture)["general"]["confirm_tab_close"],
                    "external settings loaded globally")
                expected_size = baseline + 0.5 if edit_font else 18.0
                smoke.wait_for(lambda: fixture.runtime_config()["general"]["confirm_tab_close"]
                    and fixture.runtime_config()["font"]["size"] == expected_size,
                    "untouched settings rebased while edited font size is retained")
                if edit_font:
                    fixture.tabs(5)
                else:
                    fixture.tabs(2, reverse=True)
                modified = fixture.config.stat().st_mtime_ns
                fixture.x11.chord("Return")
                smoke.wait_for(lambda: fixture.config.stat().st_mtime_ns != modified
                    and tomllib.loads(fixture.config.read_text()).get("font", {}).get("size") == expected_size,
                    "rebased settings saved")
                saved = tomllib.loads(fixture.config.read_text())
                self.assertTrue(saved["general"]["confirm_tab_close"])
                self.assertEqual(saved["font"]["normal"]["family"], "DejaVu Sans Mono")
                self.assertEqual(saved["general"]["import"], ["import.toml"])

    def test_recovery_follows_focus_between_unchanged_windows(self):
        with isolated_fixture(language.Fixture) as fixture:
            fixture.config.write_text(fixture.config.read_text().replace(
                "confirm_tab_close = false", "confirm_tab_close = false\nsession_autosave = true"))
            fixture.start()
            recovery = fixture.work / "state/termust/recovery.json"

            def saved_directory():
                try:
                    return json.loads(recovery.read_text())["tabs"][0]["working_directory"]
                except (FileNotFoundError, json.JSONDecodeError):
                    return None

            smoke.wait_for(lambda: saved_directory() == str(fixture.work), "first window autosaved")
            second_directory = fixture.work / "second"
            second_directory.mkdir()
            second = subprocess.Popen([
                fixture.binary, "--config-file", str(fixture.config),
                "--socket", str(fixture.work / "second.sock"), "--title", "second-window",
                "--working-directory", str(second_directory)], env=fixture.env,
                stdout=fixture.log, stderr=fixture.log)
            try:
                second_window = smoke.wait_for(lambda: next((window
                    for window in fixture.x11.windows()
                    if fixture.x11.title(window) == "second-window"), None), "second window opened")
                time.sleep(0.4)
                for window, directory in ((second_window, second_directory),
                                          (fixture.window, fixture.work),
                                          (second_window, second_directory)):
                    focus_window(fixture, window)
                    smoke.wait_for(lambda: saved_directory() == str(directory),
                        "recovery follows the newly focused window")
            finally:
                if second.poll() is None:
                    second.terminate()
                    second.wait(timeout=5)

    def test_tab_drag_ends_over_pane_divider(self):
        with isolated_fixture(panes.Fixture) as fixture:
            fixture.start()
            fixture.rename("A")
            fixture.x11.chord("Control_L", "Shift_L", "t")
            fixture.rename("B")
            fixture.split("e")
            fixture.focus("Left")
            left = fixture.probe("left")
            fixture.focus("Right")
            columns, _, cell_width, cell_height = fixture.dimensions()
            tab_width = min(30, max(6, (columns - 6) // 2))

            def move(x, y):
                fixture.x11.xtest.XTestFakeMotionEvent(fixture.x11.display, -1, int(x), int(y), 0)
                fixture.x11.lib.XFlush(fixture.x11.display)
                time.sleep(0.15)

            move((tab_width + 3) * cell_width, cell_height / 2)
            fixture.button(True)
            move((left["columns"] + 0.5) * cell_width, 6.5 * cell_height)
            fixture.button(False)
            move(3 * cell_width, cell_height / 2)
            self.assertEqual([tab["title"] for tab in fixture.snapshot()["tabs"]], ["A", "B"])
            fixture.drag_tab(1, 0, 2)
            self.assertEqual([tab["title"] for tab in fixture.snapshot()["tabs"]], ["B", "A"])

    def test_bell_command_runs_for_inactive_sources(self):
        with isolated_fixture(panes.Fixture) as fixture:
            marker = fixture.work / "bells"
            fixture.config.write_text(fixture.config.read_text() + '\n[bell]\ncommand = '
                + '{ program = "/bin/sh", args = ["-c", '
                + json.dumps("echo bell >> " + shlex.quote(str(marker))) + '] }\n')
            fixture.start()
            fixture.command("printf '\\a'")
            smoke.wait_for(marker.exists, "active bell command fired")
            fixture.split("e")
            fixture.focus("Left")
            for index, target in enumerate(("pane", "tab"), start=2):
                gate = fixture.work / f"bell-gate-{target}"
                done = fixture.work / f"bell-done-{target}"
                fixture.command("while [ ! -f " + shlex.quote(str(gate))
                    + ' ]; do sleep .05; done; printf "\\a"; touch ' + shlex.quote(str(done)))
                if target == "pane":
                    fixture.focus("Right")
                else:
                    fixture.x11.chord("Control_L", "Shift_L", "t")
                time.sleep(0.3)
                gate.touch()
                smoke.wait_for(done.exists, f"inactive {target} emitted bell")
                smoke.wait_for(lambda: len(marker.read_text().splitlines()) == index,
                    f"inactive {target} bell command fired")
                if target == "pane":
                    fixture.focus("Left")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--case", action="append", choices=[
        name.removeprefix("test_") for name in unittest.defaultTestLoader.getTestCaseNames(BugRegressions)])
    args = parser.parse_args()
    if args.binary:
        BINARY = str(args.binary.resolve())
    else:
        metadata = json.loads(subprocess.check_output([
            "cargo", "metadata", "--no-deps", "--format-version", "1"], text=True))
        BINARY = str(Path(metadata["target_directory"]) / "debug/termust")
    suite = unittest.TestSuite(BugRegressions("test_" + name) for name in args.case) if args.case \
        else unittest.defaultTestLoader.loadTestsFromTestCase(BugRegressions)
    raise SystemExit(not unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful())
