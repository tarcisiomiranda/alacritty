import argparse
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

spec = importlib.util.spec_from_file_location(
    "productivity_smoke", Path(__file__).with_name("test-productivity.py"))
productivity = importlib.util.module_from_spec(spec)
spec.loader.exec_module(productivity)
smoke = productivity.smoke


class Fixture(productivity.Fixture):
    def write_config(self, selected_language):
        super().write_config(selected_language)
        self.config.write_text(self.config.read_text().replace("confirm_tab_close = false\n", ""))

    def initialize(self, name):
        self.command("PS1=''; PS2='' ")
        self.rename(name)
        return self.report(name + "-pid")[0]

    def new_tab(self, name):
        self.x11.chord("Control_L", "Shift_L", "t")
        return self.initialize(name)

    def request_close(self, entry="keyboard", index=0, count=1):
        if entry == "keyboard":
            self.x11.chord("Control_L", "Shift_L", "w")
            return
        columns, _, cell_width, cell_height = self.dimensions()
        tab_width = min(30, max(6, (columns - 6) // count))
        column = index * tab_width + (tab_width - 2 if entry == "bar" else 3)
        self.x11.click((column + 0.5) * cell_width, cell_height / 2,
            {"bar": 1, "middle": 2, "context": 3}[entry])
        if entry == "context":
            left = min(column, columns - min(50, columns))
            _, rows, _, _ = self.dimensions()
            if rows >= 23:
                self.click(left + 3, 21)
            else:
                header = int(rows >= 8)
                capacity = rows - header - 2
                offset = 19 - capacity
                for _ in range(offset):
                    self.click(left + 3, 3, 5)
                self.click(left + 3, header + 1 + 18 - offset)

    def close_pixels(self, label):
        columns, rows, _, _ = self.dimensions()
        width = min(64, columns - 4)
        left, top = (columns - width) // 2, (rows - min(12, rows)) // 2
        return self.pixels(self.capture(label), left + 2, top + 1, width - 4)

    def click_decision(self, confirm):
        columns, rows, _, _ = self.dimensions()
        width = min(64, columns - 4)
        left, top = (columns - width) // 2, (rows - min(12, rows)) // 2
        self.click(left + (3 if confirm else 19), top + 8)

    def preferences_tabs(self):
        self.preferences(page="tabs")

    def toggle_confirmation(self, save):
        self.preferences_tabs()
        self.tabs(4)
        self.x11.chord("space")
        if save:
            self.tabs(2)
            self.x11.chord("Return")
            self.x11.chord("Escape")

    def gate_exit(self, label):
        ready, gate = (self.work / (label + suffix) for suffix in ("-ready", "-gate"))
        self.command(f"printf ready > {shlex.quote(str(ready))}; "
            f"while test ! -e {shlex.quote(str(gate))}; do sleep 0.05; done; exit")
        smoke.wait_for(ready.exists, label + " shell waiting for its exit gate")
        return gate

    def raw_probe(self, label):
        ready, captured, gate = (self.work / (label + suffix)
            for suffix in ("-ready", "-captured", "-gate"))
        self.command("python3 " + " ".join(shlex.quote(str(path))
            for path in (self.work / "input-probe.py", ready, captured, gate)))
        smoke.wait_for(ready.exists, label + " raw PTY probe ready")
        return captured, gate

    def finish_probe(self, probe, label):
        captured, gate = probe
        gate.write_text("release")
        smoke.wait_for(captured.exists, label + " raw PTY probe completed")
        assert captured.read_bytes() == b"", (label, captured.read_bytes())

    def hold(self, *keys):
        codes = [self.x11.keycode(key) for key in keys]
        for code in codes:
            self.x11.press(code, True)
        self.x11.lib.XFlush(self.x11.display)
        time.sleep(1.1)
        for code in reversed(codes):
            self.x11.press(code, False)
        self.x11.lib.XFlush(self.x11.display)
        time.sleep(0.15)

    def stop(self):
        if self.app is None or self.app.poll() is not None:
            return
        for _ in range(24):
            if self.app.poll() is not None:
                break
            self.x11.chord("Escape")
            confirmation = self.runtime_config()["general"]["confirm_tab_close"]
            self.request_close()
            if confirmation:
                self.x11.chord("y")
        smoke.wait_for(lambda: self.app.poll() == 0, "all confirmation fixture tabs closed")
        self.app = None


def alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False


def exited(pid, label):
    smoke.wait_for(lambda: not alive(pid), label + " shell exited")


def assert_tabs(fixture, titles, active):
    snapshot = fixture.snapshot()
    productivity.assert_titles(snapshot, titles)
    assert snapshot["active"] == active, snapshot


def keyboard_and_last_tab(fixture):
    first_pid = fixture.initialize("Alpha")
    app_pid = fixture.app.pid
    assert fixture.runtime_config()["general"]["confirm_tab_close"] is True
    assert "confirm_tab_close" not in tomllib.loads(fixture.config.read_text())["general"]
    for label, keys in [("n", ("n",)), ("N", ("Shift_L", "n")), ("escape", ("Escape",))]:
        fixture.request_close()
        fixture.capture("last-tab-cancel-" + label)
        fixture.x11.chord(*keys)
        assert alive(first_pid) and fixture.app.pid == app_pid and fixture.app.poll() is None
        assert_tabs(fixture, ["Alpha"], 0)
        assert fixture.report("cancel-" + label)[0] == first_pid
    for label, keys in [("y", ("y",)), ("Y", ("Shift_L", "y")), ("enter", ("Return",))]:
        target_pid = fixture.new_tab("Keyboard-" + label)
        fixture.request_close()
        assert alive(target_pid)
        fixture.capture("keyboard-confirm-" + label)
        fixture.x11.chord(*keys)
        exited(target_pid, label)
        assert alive(first_pid)
        assert_tabs(fixture, ["Alpha"], 0)
    fixture.stop()
    exited(first_pid, "last confirmed tab")
    fixture.start()
    print("ok: default confirmation, Y/y/Enter, N/n/Escape and final-tab protection", flush=True)


def mouse_entries(fixture):
    first_pid = fixture.initialize("Alpha")
    for entry in ("bar", "middle", "context"):
        name = "Mouse-" + entry
        target_pid = fixture.new_tab(name)
        fixture.x11.chord("Alt_L", "1")
        fixture.request_close(entry, 1, 2)
        fixture.capture(entry + "-inactive-cancel")
        fixture.click_decision(False)
        assert alive(first_pid) and alive(target_pid)
        assert_tabs(fixture, ["Alpha", name], 0)
        assert fixture.report(entry + "-active-after-cancel")[0] == first_pid
        fixture.request_close(entry, 1, 2)
        fixture.capture(entry + "-inactive-confirm")
        fixture.click_decision(True)
        exited(target_pid, entry)
        assert alive(first_pid)
        assert_tabs(fixture, ["Alpha"], 0)
    print("ok: bar close, middle-click and context close preserve inactive-target identity", flush=True)
    return first_pid


def modal_input(fixture, first_pid):
    probe = fixture.raw_probe("cancel-input")
    for label, keys in [("n", ("n",)), ("N", ("Shift_L", "n")), ("escape", ("Escape",))]:
        fixture.request_close()
        fixture.hold(*keys)
        assert alive(first_pid), label
    fixture.finish_probe(probe, "cancel-input")
    assert fixture.report("input-after-cancel")[0] == first_pid
    for label, keys in [("y", ("y",)), ("Y", ("Shift_L", "y")), ("enter", ("Return",))]:
        target_pid = fixture.new_tab("Input-" + label)
        fixture.x11.chord("Alt_L", "1")
        probe = fixture.raw_probe("confirm-input-" + label)
        fixture.request_close("bar", 1, 2)
        fixture.hold(*keys)
        exited(target_pid, "held " + label)
        fixture.finish_probe(probe, "confirm-input-" + label)
        assert_tabs(fixture, ["Alpha"], 0)
        assert fixture.report("input-after-" + label)[0] == first_pid
    print("ok: modal decisions, key releases and held-key repeats do not enter surviving PTYs", flush=True)


def asynchronous_exits(fixture, first_pid):
    target_pid = fixture.new_tab("Pending")
    survivor_pid = fixture.new_tab("Survivor")
    fixture.x11.chord("Alt_L", "1")
    other_exit = fixture.gate_exit("other-exit")
    fixture.x11.chord("Alt_L", "3")
    fixture.request_close("bar", 1, 3)
    fixture.capture("pending-before-other-exit")
    other_exit.write_text("exit")
    exited(first_pid, "background tab")
    time.sleep(0.3)
    assert alive(target_pid) and alive(survivor_pid)
    fixture.capture("pending-after-other-exit")
    fixture.x11.chord("y")
    exited(target_pid, "stable pending target")
    assert alive(survivor_pid)
    assert_tabs(fixture, ["Survivor"], 0)
    assert fixture.report("stable-target-survivor")[0] == survivor_pid
    target_pid = fixture.new_tab("Self-exiting")
    target_exit = fixture.gate_exit("target-exit")
    fixture.x11.chord("Alt_L", "1")
    fixture.request_close("middle", 1, 2)
    fixture.capture("pending-before-target-exit")
    target_exit.write_text("exit")
    exited(target_pid, "pending target")
    time.sleep(0.3)
    fixture.capture("pending-dismissed-after-target-exit")
    assert fixture.report("target-exit-survivor")[0] == survivor_pid
    assert_tabs(fixture, ["Survivor"], 0)
    target_pid = fixture.new_tab("Fresh-target")
    fixture.request_close()
    fixture.x11.chord("n")
    assert alive(target_pid) and alive(survivor_pid)
    assert_tabs(fixture, ["Survivor", "Fresh-target"], 1)
    fixture.request_close()
    fixture.x11.chord("y")
    exited(target_pid, "fresh target after automatic exit")
    assert_tabs(fixture, ["Survivor"], 0)
    print("ok: unrelated auto-exit cannot retarget; pending auto-exit dismisses without closing another tab", flush=True)
    return survivor_pid


def localization_and_compact(fixture, survivor_pid):
    fixture.refresh_size()
    columns, rows, cell_width, cell_height = fixture.dimensions()
    width, height = fixture.x11.geometry(fixture.window)
    headers = {}
    for compact in (False, True):
        if compact:
            fixture.x11.lib.XResizeWindow(fixture.x11.display, fixture.window,
                round(45 * cell_width), round(19 * cell_height))
            fixture.x11.lib.XFlush(fixture.x11.display)
            time.sleep(0.3)
            fixture.refresh_size()
            assert fixture.dimensions()[:2] == (45, 18), fixture.dimensions()
        for selected_language in ("en", "pt-BR"):
            fixture.general(language=selected_language)
            fixture.request_close()
            label = "close-" + selected_language + ("-compact" if compact else "")
            headers[(compact, selected_language)] = fixture.close_pixels(label)
            fixture.x11.chord("Escape")
            fixture.preferences_tabs()
            fixture.capture("tabs-" + selected_language + ("-compact" if compact else ""))
            fixture.x11.chord("Escape")
        assert headers[(compact, "en")] != headers[(compact, "pt-BR")]
    fixture.x11.lib.XResizeWindow(fixture.x11.display, fixture.window, width, height)
    fixture.x11.lib.XFlush(fixture.x11.display)
    time.sleep(0.3)
    fixture.refresh_size()
    assert fixture.dimensions()[:2] == (columns, rows)
    fixture.general(language="en")
    assert fixture.report("localized-survivor")[0] == survivor_pid
    print("ok: English/Portuguese close and Tabs settings panels captured at normal and 45x18 sizes", flush=True)


def saved_preferences(fixture, survivor_pid):
    original = fixture.config.read_bytes()
    fixture.toggle_confirmation(save=False)
    smoke.wait_for(lambda: fixture.runtime_config()["general"]["confirm_tab_close"] is False,
        "confirmation preference preview disabled")
    fixture.capture("tabs-draft-disabled")
    fixture.x11.chord("Escape")
    assert fixture.config.read_bytes() == original
    smoke.wait_for(lambda: fixture.runtime_config()["general"]["confirm_tab_close"] is True,
        "cancel restored confirmation preference")
    assert fixture.report("preference-cancel-survivor")[0] == survivor_pid
    fixture.toggle_confirmation(save=True)
    smoke.wait_for(lambda: tomllib.loads(fixture.config.read_text())["general"]["confirm_tab_close"] is False,
        "disabled confirmation saved")
    assert fixture.runtime_config()["general"]["confirm_tab_close"] is False
    assert fixture.report("preference-save-survivor")[0] == survivor_pid
    for entry in ("keyboard", "bar", "middle", "context"):
        target_pid = fixture.new_tab("Disabled-" + entry)
        fixture.request_close(entry, 1, 2)
        exited(target_pid, "disabled " + entry)
        assert_tabs(fixture, ["Survivor"], 0)
    fixture.stop()
    fixture.start()
    assert fixture.runtime_config()["general"]["confirm_tab_close"] is False
    restart_pid = fixture.initialize("Restart-disabled")
    fixture.request_close()
    smoke.wait_for(lambda: fixture.app.poll() == 0, "disabled confirmation closes final tab immediately after restart")
    exited(restart_pid, "disabled final tab")
    fixture.start()
    enabled_pid = fixture.initialize("Enabled")
    fixture.toggle_confirmation(save=True)
    smoke.wait_for(lambda: fixture.runtime_config()["general"]["confirm_tab_close"] is True,
        "confirmation re-enabled and saved")
    assert tomllib.loads(fixture.config.read_text())["general"]["confirm_tab_close"] is True
    fixture.request_close()
    fixture.x11.chord("n")
    assert fixture.report("enabled-survivor")[0] == enabled_pid
    fixture.stop()
    fixture.start()
    assert fixture.runtime_config()["general"]["confirm_tab_close"] is True
    defaults_pid = fixture.initialize("Defaults")
    fixture.toggle_confirmation(save=True)
    assert fixture.runtime_config()["general"]["confirm_tab_close"] is False
    fixture.preferences_tabs()
    fixture.tabs(5)
    fixture.x11.chord("Return")
    fixture.capture("tabs-defaults-restored")
    fixture.tabs(1)
    fixture.x11.chord("Return")
    fixture.x11.chord("Escape")
    smoke.wait_for(lambda: fixture.runtime_config()["general"]["confirm_tab_close"] is True,
        "Defaults restored enabled confirmation")
    assert tomllib.loads(fixture.config.read_text())["general"]["confirm_tab_close"] is True
    fixture.refresh_size()
    fixture.request_close()
    fixture.x11.chord("n")
    assert fixture.report("defaults-survivor")[0] == defaults_pid
    print("ok: preference preview/cancel preserves PID; save/restart, disable/re-enable and Defaults persist", flush=True)


def run(binary, screenshot):
    for executable in ("Xvfb", "import", "convert", "python3", binary):
        if not shutil.which(executable):
            raise RuntimeError("Missing executable: " + executable)
    with tempfile.TemporaryDirectory(prefix="alacritty-close-confirmation-") as temporary:
        fixture = Fixture(binary, Path(temporary), screenshot)
        (fixture.work / "input-probe.py").write_text('''import os
from pathlib import Path
import select
import sys
import termios
import time
import tty

ready, captured, gate = map(Path, sys.argv[1:])
fd = sys.stdin.fileno()
old = termios.tcgetattr(fd)
data = bytearray()
tty.setraw(fd)
os.write(sys.stdout.fileno(), b"\\x1b[>3u")
ready.write_text("ready")
try:
    deadline = time.monotonic() + 20
    while not gate.exists() and time.monotonic() < deadline:
        if select.select([fd], [], [], 0.05)[0]:
            data.extend(os.read(fd, 4096))
    while select.select([fd], [], [], 0.15)[0]:
        data.extend(os.read(fd, 4096))
finally:
    os.write(sys.stdout.fileno(), b"\\x1b[<u")
    termios.tcsetattr(fd, termios.TCSANOW, old)
    captured.write_bytes(data)
''')
        try:
            fixture.start()
            keyboard_and_last_tab(fixture)
            first_pid = mouse_entries(fixture)
            modal_input(fixture, first_pid)
            survivor_pid = asynchronous_exits(fixture, first_pid)
            localization_and_compact(fixture, survivor_pid)
            saved_preferences(fixture, survivor_pid)
            fixture.stop()
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
