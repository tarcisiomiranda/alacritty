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

spec = importlib.util.spec_from_file_location("language_smoke", Path(__file__).with_name("test-language.py"))
language = importlib.util.module_from_spec(spec)
spec.loader.exec_module(language)
smoke = language.smoke


class Fixture(language.Fixture):
    def write_config(self, selected_language):
        super().write_config(selected_language)
        self.config.write_text(self.config.read_text().replace(
            "[general]\n", f"[general]\nworking_directory = {json.dumps(str(self.work))}\n"))

    def start(self):
        before = set(self.x11.windows())
        self.app = subprocess.Popen([self.binary, "--config-file", str(self.config),
            "--socket", str(self.socket)], env=dict(self.env, LANG="C"),
            stdout=self.log, stderr=self.log)
        self.window = smoke.wait_for(lambda: next((window for window in self.x11.windows()
            if window not in before and self.x11.title(window)), None), "productivity window opened")
        time.sleep(0.4)
        self.x11.lib.XSetInputFocus(self.x11.display, self.window, 2, 0)
        self.x11.lib.XFlush(self.x11.display)
        self.size.unlink(missing_ok=True)
        self.command(f"stty size > {shlex.quote(str(self.size))}")
        smoke.wait_for(self.size.exists, "restored shell initialized")
        self.runtime_config()

    @property
    def state_directory(self):
        return self.work / "state" / "alacritty"

    def runtime_config(self):
        result = subprocess.run([self.binary, "msg", "--socket", str(self.socket),
            "get-config", "--window-id", str(self.window)], env=self.env,
            capture_output=True, text=True, check=True, timeout=5)
        return json.loads(result.stdout)

    def refresh_size(self):
        self.size.unlink(missing_ok=True)
        self.command(f"stty size > {shlex.quote(str(self.size))}")
        smoke.wait_for(self.size.exists, "terminal geometry refreshed")

    def drag_tab(self, source, target, count):
        columns, _, cell_width, cell_height = self.dimensions()
        tab_width = min(30, max(6, (columns - 6) // count))
        source_x, target_x = ((index * tab_width + 3) * cell_width for index in (source, target))
        test = self.x11.xtest
        test.XTestFakeMotionEvent(self.x11.display, -1, int(source_x), int(cell_height / 2), 0)
        test.XTestFakeButtonEvent(self.x11.display, 1, True, 0)
        self.x11.lib.XFlush(self.x11.display)
        time.sleep(0.1)
        test.XTestFakeMotionEvent(self.x11.display, -1, int(target_x), int(cell_height / 2), 0)
        self.x11.lib.XFlush(self.x11.display)
        time.sleep(0.1)
        test.XTestFakeButtonEvent(self.x11.display, 1, False, 0)
        self.x11.lib.XFlush(self.x11.display)
        time.sleep(0.1)

    def tabs(self, count, reverse=False):
        for _ in range(count):
            self.x11.chord(*(["Shift_L", "Tab"] if reverse else ["Tab"]))

    def rename(self, name):
        self.x11.chord("Control_L", "Shift_L", "r")
        self.x11.chord("Control_L", "a")
        self.x11.type(name)
        self.x11.chord("Return")
        smoke.wait_for(lambda: self.x11.title(self.window) == name, f"tab renamed to {name}")

    def toggle_replay(self):
        self.x11.chord("Control_L", "Shift_L", "r")
        self.tabs(1)
        self.x11.chord("space")
        self.tabs(1, reverse=True)
        self.x11.chord("Return")

    def report(self, label):
        path = self.work / label
        path.unlink(missing_ok=True)
        self.command(f"printf '%s\\n%s\\n' \"$$\" \"$PWD\" > {shlex.quote(str(path))}")
        smoke.wait_for(path.exists, f"{label} shell PID and working directory recorded")
        pid, cwd = path.read_text().splitlines()
        return int(pid), cwd

    def snapshot(self):
        path = self.state_directory / "tabs.json"
        previous = path.stat().st_mtime_ns if path.exists() else None
        self.x11.chord("Control_L", "Shift_L", "s")
        smoke.wait_for(lambda: path.exists() and path.stat().st_mtime_ns != previous,
            "manual snapshot updated")
        self.x11.chord("Escape")
        return json.loads(path.read_text())

    def workspace_names(self):
        path = self.state_directory / "workspaces.json"
        return sorted(json.loads(path.read_text())["workspaces"]) if path.exists() else []

    def save_workspace(self, name):
        names = self.workspace_names()
        self.x11.chord("Control_L", "Shift_L", "p")
        self.x11.type(name)
        self.tabs(len(names) + 1)
        self.x11.chord("Return")
        smoke.wait_for(lambda: name in self.workspace_names(), f"workspace {name} saved")
        self.x11.chord("Escape")

    def workspace_action(self, name, action):
        names = self.workspace_names()
        index = names.index(name)
        self.x11.chord("Control_L", "Shift_L", "p")
        self.tabs(index + 1)
        self.x11.chord("Return")
        self.tabs(len(names) - index + {"save": 0, "open": 1, "startup": 2, "delete": 3}[action])
        self.x11.chord("Return")
        if action == "delete":
            self.x11.chord("Return")
        self.x11.chord("Escape")

    def preferences(self, page="appearance"):
        self.x11.chord("Control_L", "comma")
        index = ["appearance", "terminal", "tabs", "sessions", "language", "shortcuts"].index(page)
        if index:
            self.tabs(index)
            self.x11.chord("Return")

    def general(self, **values):
        text = self.config.read_text()
        start = text.index("[general]\n") + len("[general]\n")
        end = text.find("\n[", start)
        end = len(text) if end < 0 else end
        lines = [line for line in text[start:end].splitlines()
            if line.split("=", 1)[0].strip() not in values]
        lines.extend(f"{key} = {json.dumps(value)}" for key, value in values.items() if value is not None)
        self.config.write_text(text[:start] + "\n".join(lines) + "\n" + text[end:])
        smoke.wait_for(lambda: all(self.runtime_config()["general"].get(key) == value
            for key, value in values.items()), "startup preferences reloaded")

    def stop(self, crash=False):
        if self.app is None or self.app.poll() is not None:
            return
        if crash:
            self.app.terminate()
            self.app.wait(timeout=5)
        else:
            for _ in range(24):
                if self.app.poll() is not None:
                    break
                self.x11.chord("Escape")
                self.x11.chord("Control_L", "Shift_L", "w")
            smoke.wait_for(lambda: self.app.poll() == 0, "all fixture tabs closed")
        self.app = None


def activity_and_drag(fixture, first_pid, second_pid, reopened_pid):
    fixture.refresh_size()
    fixture.x11.chord("Alt_L", "1")
    fixture.x11.chord("Alt_L", "2")
    time.sleep(0.6)
    columns, _, _, _ = fixture.dimensions()
    tab_width = min(30, max(6, (columns - 6) // 3))
    idle = fixture.pixels(fixture.capture("activity-idle"), 0, -1, tab_width)
    started, gate, done = (fixture.work / name for name in ("activity-started", "activity-gate", "activity-done"))
    fixture.x11.chord("Alt_L", "1")
    fixture.command(f"printf started > {shlex.quote(str(started))}; "
        f"while test ! -e {shlex.quote(str(gate))}; do sleep 0.05; done; "
        f"printf done > {shlex.quote(str(done))}; false")
    smoke.wait_for(started.exists, "background command entered its running phase")
    fixture.x11.chord("Alt_L", "2")
    time.sleep(0.6)
    busy = fixture.pixels(fixture.capture("activity-running"), 0, -1, tab_width)
    assert busy != idle, "Running marker did not appear in the background tab"
    assert not done.exists()
    gate.write_text("release")
    smoke.wait_for(done.exists, "background command completed with a failure")
    time.sleep(0.6)
    failed = fixture.pixels(fixture.capture("activity-failed"), 0, -1, tab_width)
    assert failed not in (idle, busy), "Failure marker did not replace the running marker"
    fixture.x11.chord("Alt_L", "1")
    fixture.x11.chord("Alt_L", "2")
    time.sleep(0.6)
    acknowledged = fixture.pixels(fixture.capture("activity-acknowledged"), 0, -1, tab_width)
    assert acknowledged == idle, "Selecting the completed tab did not clear its marker"
    fixture.drag_tab(2, 0, 3)
    snapshot = fixture.snapshot()
    assert_titles(snapshot, ["Gamma", "Alpha", "Beta"])
    assert snapshot["active"] == 0
    assert fixture.report("dragged-pid")[0] == reopened_pid
    fixture.drag_tab(0, 2, 3)
    snapshot = fixture.snapshot()
    assert_titles(snapshot, ["Alpha", "Beta", "Gamma"])
    assert snapshot["active"] == 2
    assert fixture.report("dragged-back-pid")[0] == reopened_pid
    os.kill(first_pid, 0)
    os.kill(second_pid, 0)
    print("ok: background running/failure/acknowledgement markers; mouse dragging preserves shell identity", flush=True)


def tab_bar(fixture):
    original = fixture.config.read_bytes()
    original_pid = fixture.report("tabbar-original-pid")[0]
    original_title = fixture.x11.title(fixture.window)
    fixture.rename("Terminal")
    fixture.x11.chord("Control_L", "Shift_L", "t")
    fixture.rename("Servidor")
    fixture.x11.chord("Control_L", "Shift_L", "t")
    subprocess.run([fixture.binary, "msg", "--socket", str(fixture.socket), "config",
        "--window-id", str(fixture.window), "window.dynamic_title=true"],
        env=fixture.env, capture_output=True, text=True, check=True, timeout=5)
    title = "Produção 日本語 — ambiente de desenvolvimento"
    encoded = "".join(f"\\{byte:03o}" for byte in ("\x1b[2J\x1b[H\x1b]0;" + title + "\x07").encode())
    fixture.command(f"printf '{encoded}'")
    smoke.wait_for(lambda: fixture.x11.title(fixture.window) == title,
        "long Unicode shell title reached the tab bar")
    fixture.refresh_size()
    columns, rows, cell_width, cell_height = fixture.dimensions()
    tab_width = min(30, max(6, (columns - 6) // 3))

    def hover(column, line=0.5):
        fixture.x11.xtest.XTestFakeMotionEvent(fixture.x11.display, -1,
            round((column + 0.5) * cell_width), round(line * cell_height), 0)
        fixture.x11.lib.XFlush(fixture.x11.display)
        time.sleep(0.15)

    def capture(label):
        path = fixture.capture(label)
        _, _, _, cell_height = fixture.dimensions()
        width, _ = fixture.x11.geometry(fixture.window)
        cropped = fixture.work / (label + "-bar.png")
        subprocess.run(["convert", str(path), "-crop",
            f"{width}x{round(cell_height)}+0+0", "+repage", str(cropped)],
            capture_output=True, check=True, timeout=5)
        if fixture.screenshot:
            destination = fixture.screenshot.with_name(fixture.screenshot.stem + "." +
                label + "-bar" + fixture.screenshot.suffix)
            shutil.copyfile(cropped, destination)
        return path

    hover(40, 5)
    idle = capture("tabbar-dark-unicode")
    hover(7)
    hovered = capture("tabbar-hover")
    assert fixture.pixels(idle, 0, -1, tab_width) != fixture.pixels(
        hovered, 0, -1, tab_width), "Inactive tab did not show hover feedback"
    hover(tab_width - 2.5)
    closing = capture("tabbar-close-hover")
    assert fixture.pixels(hovered, tab_width - 3, -1, 2) != fixture.pixels(
        closing, tab_width - 3, -1, 2), "Close target did not show separate hover feedback"
    snapshot = fixture.snapshot()
    assert len(snapshot["tabs"]) == 3 and snapshot["active"] == 2
    fixture.x11.click((tab_width - 0.5) * cell_width, cell_height / 2)
    snapshot = fixture.snapshot()
    assert len(snapshot["tabs"]) == 3 and snapshot["active"] == 0, "Tab margin hit the close button"
    fixture.x11.chord("Alt_L", "3")

    def greens(pixels):
        return sum(green > red + 5 and green > blue + 5
            for red, green, blue in zip(pixels[0::3], pixels[1::3], pixels[2::3]))

    def menu_accent(label):
        left, top = fixture.menu()
        fixture.x11.chord("Down")
        path = fixture.capture(label)
        pixels = fixture.pixels(path, left, top, min(50, columns), height=min(23, rows))
        fixture.x11.chord("Escape")
        return pixels

    menu_before = menu_accent("menu-theme-original")
    subprocess.run([fixture.binary, "msg", "--socket", str(fixture.socket), "config",
        "--window-id", str(fixture.window), 'colors.normal.blue="#30b060"'],
        env=fixture.env, capture_output=True, text=True, check=True, timeout=5)
    hover(40, 5)
    themed = capture("tabbar-themed-green")
    assert greens(fixture.pixels(themed, tab_width * 2, -1, tab_width)) > (
        greens(fixture.pixels(idle, tab_width * 2, -1, tab_width)) + 20), "Tab accent ignored the theme color"
    assert greens(menu_accent("menu-themed-green")) > greens(menu_before) + 20, "Menu accent ignored the theme color"
    subprocess.run([fixture.binary, "msg", "--socket", str(fixture.socket), "config",
        "--window-id", str(fixture.window), 'colors.primary.background="#f5f7fb"',
        'colors.primary.foreground="#202b3d"'], env=fixture.env,
        capture_output=True, text=True, check=True, timeout=5)
    hover(40, 5)
    capture("tabbar-light-unicode")
    width, height = fixture.x11.geometry(fixture.window)
    fixture.x11.lib.XResizeWindow(fixture.x11.display, fixture.window,
        round(45 * cell_width), round(19 * cell_height))
    fixture.x11.lib.XFlush(fixture.x11.display)
    time.sleep(0.3)
    fixture.refresh_size()
    assert fixture.dimensions()[:2] == (45, 18)
    capture("tabbar-light-compact")
    fixture.x11.lib.XResizeWindow(fixture.x11.display, fixture.window, width, height)
    fixture.x11.lib.XFlush(fixture.x11.display)
    time.sleep(0.3)
    fixture.refresh_size()
    assert fixture.dimensions()[:2] == (columns, rows)
    subprocess.run([fixture.binary, "msg", "--socket", str(fixture.socket), "config",
        "--window-id", str(fixture.window), "--reset"], env=fixture.env,
        capture_output=True, text=True, check=True, timeout=5)
    fixture.x11.chord("Control_L", "Shift_L", "w")
    fixture.x11.chord("Control_L", "Shift_L", "w")
    fixture.rename(original_title)
    assert fixture.report("tabbar-restored-pid")[0] == original_pid
    assert len(fixture.snapshot()["tabs"]) == 1
    assert fixture.config.read_bytes() == original
    print("ok: compact tab bar, close margin, theme accents in tabs/menus, Unicode and light layout", flush=True)

def settings_navigation(fixture):
    original = fixture.config.read_bytes()
    hidden = fixture.runtime_config()["mouse"]["hide_when_typing"]
    columns, rows, cell_width, cell_height = fixture.dimensions()
    assert columns >= 90 and rows >= 26
    panel_width = min(104, columns - 4)
    panel_height = min(30, rows - 2)
    left, top = (columns - panel_width) // 2, (rows - panel_height) // 2
    fixture.preferences()
    fixture.click(left + 10, top + 7)
    fixture.capture("settings-sidebar")
    fixture.click(left + 30, top + 8)
    smoke.wait_for(lambda: fixture.runtime_config()["mouse"]["hide_when_typing"] != hidden,
        "sidebar mouse selection reached terminal controls")
    fixture.click(left + panel_width - 15, top + panel_height - 3)
    smoke.wait_for(lambda: fixture.runtime_config()["mouse"]["hide_when_typing"] == hidden,
        "anchored Cancel restored the preview")
    assert fixture.config.read_bytes() == original
    width, height = fixture.x11.geometry(fixture.window)
    fixture.x11.lib.XResizeWindow(fixture.x11.display, fixture.window,
        round(45 * cell_width), round(19 * cell_height))
    fixture.x11.lib.XFlush(fixture.x11.display)
    time.sleep(0.3)
    fixture.refresh_size()
    assert fixture.dimensions()[:2] == (45, 18)
    fixture.preferences()
    before = fixture.capture("settings-overflow-top")
    body = fixture.pixels(before, 4, 7, 34)
    footer = fixture.pixels(before, 4, 15, 37)
    fixture.click(20, 10, button=5)
    after = fixture.capture("settings-overflow-scrolled")
    assert fixture.pixels(after, 4, 7, 34) != body, "Mouse wheel did not scroll the settings body"
    assert fixture.pixels(after, 4, 15, 37) == footer, "Footer moved during body scrolling"
    fixture.click(40, 7)
    restored = fixture.capture("settings-overflow-arrow")
    assert fixture.pixels(restored, 4, 7, 34) == body
    fixture.x11.chord("Page_Down")
    assert fixture.pixels(fixture.capture("settings-overflow-page-down"), 4, 7, 34) != body
    fixture.x11.chord("Page_Up")
    assert fixture.pixels(fixture.capture("settings-overflow-page-up"), 4, 7, 34) == body
    fixture.x11.chord("Escape")
    assert fixture.config.read_bytes() == original
    fixture.x11.lib.XResizeWindow(fixture.x11.display, fixture.window, width, height)
    fixture.x11.lib.XFlush(fixture.x11.display)
    time.sleep(0.3)
    fixture.refresh_size()
    print("ok: sidebar mouse navigation, anchored Cancel, wheel/arrows/PageUpDown and fixed footer", flush=True)

def localized_panels(fixture):
    fixture.general(language="pt-BR")
    for page in ["appearance", "terminal", "tabs", "sessions", "language", "shortcuts"]:
        fixture.preferences(page)
        fixture.capture(page + "-pt")
        fixture.x11.chord("Escape")
    fixture.x11.chord("Control_L", "Shift_L", "p")
    fixture.capture("workspaces-pt")
    fixture.x11.chord("Escape")
    fixture.refresh_size()
    columns, rows, cell_width, cell_height = fixture.dimensions()
    width, height = fixture.x11.geometry(fixture.window)
    fixture.x11.lib.XResizeWindow(fixture.x11.display, fixture.window,
        round(45 * cell_width), round(19 * cell_height))
    fixture.x11.lib.XFlush(fixture.x11.display)
    time.sleep(0.3)
    fixture.refresh_size()
    compact_columns, compact_rows, _, _ = fixture.dimensions()
    assert (compact_columns, compact_rows) == (45, 18), (compact_columns, compact_rows)
    for page in ["appearance", "terminal", "tabs", "sessions", "language", "shortcuts"]:
        fixture.preferences(page)
        fixture.capture(page + "-pt-compact")
        fixture.x11.chord("Escape")
    fixture.x11.chord("Control_L", "Shift_L", "p")
    fixture.capture("workspaces-pt-compact")
    fixture.x11.chord("Escape")
    fixture.general(language="en")
    for page in ["appearance", "terminal", "tabs", "sessions", "language", "shortcuts"]:
        fixture.preferences(page)
        fixture.capture(page + "-en-compact")
        fixture.x11.chord("Escape")
    fixture.x11.lib.XResizeWindow(fixture.x11.display, fixture.window, width, height)
    fixture.x11.lib.XFlush(fixture.x11.display)
    time.sleep(0.3)
    fixture.refresh_size()
    assert fixture.dimensions()[:2] == (columns, rows)
    fixture.general(language="en")
    for page in ["appearance", "terminal", "tabs", "sessions", "language", "shortcuts"]:
        fixture.preferences(page)
        fixture.capture(page + "-en")
        fixture.x11.chord("Escape")
    fixture.preferences()
    fixture.tabs(11)
    fixture.x11.chord("Return")
    fixture.x11.chord("Return")
    fixture.capture("appearance-en-light")
    fixture.tabs(6, reverse=True)
    fixture.x11.chord("Return")
    fixture.capture("shortcuts-en-light")
    fixture.x11.lib.XResizeWindow(fixture.x11.display, fixture.window,
        round(45 * cell_width), round(19 * cell_height))
    fixture.x11.lib.XFlush(fixture.x11.display)
    time.sleep(0.3)
    fixture.capture("shortcuts-en-light-compact")
    fixture.x11.chord("Escape")
    fixture.x11.lib.XResizeWindow(fixture.x11.display, fixture.window, width, height)
    fixture.x11.lib.XFlush(fixture.x11.display)
    time.sleep(0.3)
    fixture.refresh_size()
    print("ok: six pages in English/Portuguese, 45x18 viewport and light theme", flush=True)

def assert_titles(snapshot, expected):
    assert [tab["title"] for tab in snapshot["tabs"]] == expected, snapshot


def shortcuts(fixture):
    marker = fixture.work / "custom-shortcut"
    command = f"printf 'custom\\n' >> {shlex.quote(str(marker))}\n"
    fixture.config.write_text(fixture.config.read_text() +
        '\n[general.shortcuts]\nFutureAction = "F2"\n[keyboard]\nbindings = [{ key = "F8", mods = "Control|Shift", chars = ' +
        json.dumps(command) + ' }] # retain custom binding\n')
    smoke.wait_for(lambda: fixture.runtime_config()["general"]["shortcuts"].get("FutureAction") == "F2",
        "custom shortcut fixture reloaded")
    custom_bindings = tomllib.loads(fixture.config.read_text())["keyboard"]["bindings"]
    original = fixture.config.read_bytes()
    fixture.x11.chord("Control_L", "Shift_L", "F8")
    smoke.wait_for(marker.exists, "unrelated custom shortcut executed")
    first_pid = fixture.report("shortcut-original-pid")[0]

    fixture.preferences("shortcuts")
    fixture.tabs(1)
    fixture.x11.chord("Return")
    fixture.x11.chord("Control_L", "Shift_L", "d")
    fixture.capture("shortcut-conflict")
    fixture.x11.chord("Escape")
    fixture.capture("shortcut-capture-cancelled")
    fixture.x11.chord("Escape")
    assert fixture.config.read_bytes() == original
    assert len(fixture.snapshot()["tabs"]) == 1, "Conflicting capture leaked to DuplicateTab"

    fixture.preferences("shortcuts")
    fixture.tabs(1)
    fixture.x11.chord("Return")
    fixture.x11.chord("Control_L", "Shift_L", "F9")
    fixture.capture("shortcut-draft")
    assert "CreateNewTab" not in fixture.runtime_config()["general"]["shortcuts"]
    assert fixture.config.read_bytes() == original
    fixture.x11.chord("Escape")
    assert fixture.config.read_bytes() == original
    assert fixture.report("shortcut-cancelled-pid")[0] == first_pid

    fixture.preferences("shortcuts")
    fixture.tabs(1)
    fixture.x11.chord("Return")
    fixture.x11.chord("Control_L", "Shift_L", "F9")
    fixture.tabs(5)
    fixture.x11.chord("Return")
    smoke.wait_for(lambda: fixture.runtime_config()["general"]["shortcuts"].get("CreateNewTab") == "Ctrl+Shift+F9",
        "new tab shortcut saved and applied")
    fixture.x11.chord("Escape")
    saved = tomllib.loads(fixture.config.read_text())
    assert saved["keyboard"]["bindings"] == custom_bindings
    assert saved["general"]["import"] == ["import.toml"]
    assert saved["general"]["shortcuts"]["FutureAction"] == "F2"
    assert "# retain custom binding" in fixture.config.read_text()
    fixture.x11.chord("Control_L", "Shift_L", "F9")
    assert fixture.report("shortcut-remapped-pid")[0] != first_pid
    assert len(fixture.snapshot()["tabs"]) == 2
    fixture.x11.chord("Control_L", "Shift_L", "t")
    assert len(fixture.snapshot()["tabs"]) == 2, "Old shortcut remained active"
    fixture.x11.chord("Control_L", "Shift_L", "w")
    assert len(fixture.snapshot()["tabs"]) == 1
    fixture.stop()
    fixture.start()
    assert fixture.runtime_config()["general"]["shortcuts"]["CreateNewTab"] == "Ctrl+Shift+F9"
    fixture.x11.chord("Control_L", "Shift_L", "F9")
    assert len(fixture.snapshot()["tabs"]) == 2
    fixture.x11.chord("Control_L", "Shift_L", "w")
    assert len(fixture.snapshot()["tabs"]) == 1
    fixture.x11.chord("Control_L", "Shift_L", "F8")
    smoke.wait_for(lambda: len(marker.read_text().splitlines()) == 2, "custom binding survived shortcut save and restart")

    fixture.preferences("shortcuts")
    fixture.tabs(5)
    fixture.x11.chord("Return")
    fixture.tabs(1)
    fixture.x11.chord("Return")
    smoke.wait_for(lambda: fixture.runtime_config()["general"]["shortcuts"].get("CreateNewTab") == "",
        "shortcut explicitly disabled")
    fixture.x11.chord("Escape")
    fixture.x11.chord("Control_L", "Shift_L", "F9")
    fixture.x11.chord("Control_L", "Shift_L", "t")
    assert len(fixture.snapshot()["tabs"]) == 1
    fixture.x11.chord("Control_L", "u")

    fixture.preferences()
    fixture.tabs(12)
    fixture.x11.chord("Return")
    fixture.tabs(1)
    fixture.x11.chord("Return")
    smoke.wait_for(lambda: fixture.runtime_config()["general"]["shortcuts"].get("CreateNewTab") == "Ctrl+Shift+T",
        "restore defaults reinstated native shortcuts")
    fixture.x11.chord("Escape")
    fixture.general(confirm_tab_close=False)
    fixture.refresh_size()
    saved = tomllib.loads(fixture.config.read_text())
    assert saved["keyboard"]["bindings"] == custom_bindings
    assert saved["general"]["shortcuts"]["FutureAction"] == "F2"
    fixture.x11.chord("Control_L", "Shift_L", "t")
    assert len(fixture.snapshot()["tabs"]) == 2
    fixture.x11.chord("Control_L", "Shift_L", "w")
    fixture.x11.chord("Control_L", "Shift_L", "F9")
    assert len(fixture.snapshot()["tabs"]) == 1
    fixture.x11.chord("Control_L", "u")
    fixture.x11.chord("Control_L", "Shift_L", "F8")
    smoke.wait_for(lambda: len(marker.read_text().splitlines()) == 3, "custom binding survived restore defaults")
    print("ok: shortcut capture/conflicts/cancel, remap/save/restart, disable/defaults and custom bindings", flush=True)

def terminal_settings(fixture):
    baseline = fixture.runtime_config()
    original = fixture.config.read_bytes()
    history = baseline["scrolling"]["history"]
    hidden = baseline["mouse"]["hide_when_typing"]
    copied = baseline["selection"]["save_to_clipboard"]
    fixture.preferences("terminal")
    fixture.tabs(5)
    fixture.x11.chord("Return")
    fixture.tabs(2)
    fixture.x11.chord("space")
    fixture.tabs(1)
    fixture.x11.chord("space")
    smoke.wait_for(lambda: fixture.runtime_config()["mouse"]["hide_when_typing"] != hidden
        and fixture.runtime_config()["selection"]["save_to_clipboard"] != copied,
        "native terminal preference draft previewed")
    assert fixture.runtime_config()["scrolling"]["history"] == history
    assert fixture.config.read_bytes() == original
    assert super(Fixture, fixture).runtime_config()["mouse"]["hide_when_typing"] == hidden
    fixture.x11.chord("Escape")
    smoke.wait_for(lambda: fixture.runtime_config()["mouse"]["hide_when_typing"] == hidden
        and fixture.runtime_config()["selection"]["save_to_clipboard"] == copied,
        "cancel restored native terminal preferences")
    assert fixture.runtime_config()["scrolling"]["history"] == history
    assert fixture.config.read_bytes() == original
    fixture.preferences("terminal")
    fixture.tabs(5)
    fixture.x11.chord("Return")
    fixture.tabs(2)
    fixture.x11.chord("space")
    fixture.tabs(1)
    fixture.x11.chord("space")
    fixture.tabs(2)
    fixture.x11.chord("Return")
    smoke.wait_for(lambda: fixture.runtime_config()["scrolling"]["history"] == max(0, history - 1000),
        "scrollback limit applied only after saving")
    saved = tomllib.loads(fixture.config.read_text())
    assert saved["scrolling"]["history"] == max(0, history - 1000)
    assert saved["mouse"]["hide_when_typing"] != hidden
    assert saved["selection"]["save_to_clipboard"] != copied
    fixture.x11.chord("Escape")
    fixture.stop()
    fixture.start()
    assert fixture.runtime_config()["scrolling"]["history"] == max(0, history - 1000)
    assert fixture.runtime_config()["mouse"]["hide_when_typing"] != hidden
    assert fixture.runtime_config()["selection"]["save_to_clipboard"] != copied
    print("ok: native terminal settings preview/cancel/save/restart; scrollback retained until save", flush=True)

def settings(fixture):
    baseline = fixture.runtime_config()
    original = fixture.config.read_bytes()
    fixture.preferences()
    fixture.tabs(8)
    fixture.x11.chord("Return")
    smoke.wait_for(lambda: fixture.runtime_config()["font"]["size"] == baseline["font"]["size"] + 0.5,
        "font size preview applied without saving")
    assert fixture.config.read_bytes() == original
    assert super(Fixture, fixture).runtime_config()["font"]["size"] == baseline["font"]["size"]
    fixture.x11.chord("Escape")
    smoke.wait_for(lambda: fixture.runtime_config()["font"]["size"] == baseline["font"]["size"],
        "cancel restored font size")
    assert fixture.config.read_bytes() == original

    fixture.preferences()
    fixture.tabs(6)
    fixture.x11.chord("Control_L", "a")
    fixture.x11.type("monospace")
    fixture.tabs(2)
    fixture.x11.chord("Return")
    fixture.tabs(1)
    fixture.x11.chord("Return")
    fixture.tabs(2)
    fixture.x11.chord("Return")
    fixture.tabs(2)
    fixture.x11.chord("Return")
    smoke.wait_for(lambda: tomllib.loads(fixture.config.read_text()).get("font", {}).get("size")
        == baseline["font"]["size"] + 0.5, "appearance preferences saved")
    persisted = tomllib.loads(fixture.config.read_text())
    assert persisted["font"]["normal"]["family"] == "monospace"
    assert abs(persisted["window"]["opacity"] - 0.95) < 0.001
    assert persisted["general"]["import"] == ["import.toml"]
    assert persisted["env"]["LANGUAGE_TEST"] == "preserved"
    assert "# Language test fixture: retain this comment." in fixture.config.read_text()
    assert "# Retain the language comment." in fixture.config.read_text()
    fixture.x11.chord("Escape")
    fixture.stop()
    fixture.start()
    assert fixture.runtime_config()["font"]["size"] == baseline["font"]["size"] + 0.5
    assert abs(fixture.runtime_config()["window"]["opacity"] - 0.95) < 0.001
    fixture.preferences()
    fixture.capture("appearance-saved")
    fixture.x11.chord("Escape")
    print("ok: preview/cancel, appearance save/restart and unrelated configuration retained", flush=True)


def readonly_startup_workspace(fixture):
    workspace_file = fixture.state_directory / "workspaces.json"
    config_before = fixture.config.read_bytes()
    workspaces_before = workspace_file.read_bytes()
    fixture.config.chmod(0o400)
    try:
        fixture.workspace_action("Backend", "delete")
        time.sleep(0.3)
        assert fixture.config.read_bytes() == config_before
        assert workspace_file.read_bytes() == workspaces_before
        assert fixture.runtime_config()["general"]["startup_session"] == "Backend"
        assert fixture.workspace_names() == ["Backend"]
    finally:
        fixture.config.chmod(0o600)
    print("ok: failed startup preference save preserves the workspace and read-only configuration", flush=True)

def tab_workspaces(fixture):
    first, second = fixture.work / "first", fixture.work / "second"
    first.mkdir()
    second.mkdir()
    fixture.command(f"cd {shlex.quote(str(first))}")
    first_pid, first_cwd = fixture.report("first-pid")
    assert first_cwd == str(first)
    fixture.rename("Alpha")
    fixture.x11.chord("Control_L", "Shift_L", "t")
    fixture.command(f"cd {shlex.quote(str(second))}")
    second_pid, second_cwd = fixture.report("second-pid")
    assert second_cwd == str(second)
    fixture.rename("Beta")
    marker = second / "execution-count"
    fixture.command("printf 'once\\n' >> execution-count")
    smoke.wait_for(marker.exists, "source command executed once")
    fixture.toggle_replay()
    fixture.x11.chord("Control_L", "Shift_L", "d")
    duplicate_pid, duplicate_cwd = fixture.report("duplicate-pid")
    assert duplicate_pid != second_pid and duplicate_cwd == str(second)
    assert marker.read_text().splitlines() == ["once"]
    fixture.rename("Gamma")
    snapshot = fixture.snapshot()
    assert len(snapshot["tabs"]) == 3
    assert snapshot["tabs"][2]["replay_last_command"] is False
    fixture.x11.chord("Control_L", "Shift_L", "w")
    fixture.x11.chord("Control_L", "Shift_L", "Alt_L", "t")
    reopened_pid, reopened_cwd = fixture.report("reopened-pid")
    assert reopened_pid not in (first_pid, second_pid, duplicate_pid)
    assert reopened_cwd == str(second)
    assert marker.read_text().splitlines() == ["once"]
    fixture.x11.chord("Control_L", "Shift_L", "Page_Up")
    assert_titles(fixture.snapshot(), ["Alpha", "Gamma", "Beta"])
    fixture.x11.chord("Control_L", "Shift_L", "Page_Down")
    assert_titles(fixture.snapshot(), ["Alpha", "Beta", "Gamma"])
    activity_and_drag(fixture, first_pid, second_pid, reopened_pid)
    fixture.x11.chord("Alt_L", "2")
    fixture.toggle_replay()
    fixture.x11.chord("Alt_L", "3")
    fixture.save_workspace("Backend")
    workspace_file = fixture.state_directory / "workspaces.json"
    workspace = json.loads(workspace_file.read_text())["workspaces"]["Backend"]
    assert_titles(workspace, ["Alpha", "Beta", "Gamma"])
    assert [tab["working_directory"] for tab in workspace["tabs"]] == [str(first), str(second), str(second)]
    assert not any(tab["replay_last_command"] for tab in workspace["tabs"])
    assert workspace_file.stat().st_mode & 0o777 == 0o600
    fixture.workspace_action("Backend", "open")
    assert_titles(fixture.snapshot(), ["Alpha", "Beta", "Gamma", "Alpha", "Beta", "Gamma"])
    os.kill(first_pid, 0)
    os.kill(second_pid, 0)
    os.kill(reopened_pid, 0)
    before_startup = tomllib.loads(fixture.config.read_text())
    subprocess.run([fixture.binary, "msg", "--socket", str(fixture.socket), "config",
        "--window-id", str(fixture.window), "font.size=19.0", "window.opacity=0.72"],
        env=fixture.env, capture_output=True, text=True, check=True, timeout=5)
    smoke.wait_for(lambda: fixture.runtime_config()["font"]["size"] == 19.0
        and abs(fixture.runtime_config()["window"]["opacity"] - 0.72) < 0.001,
        "temporary appearance overrides applied to this window")
    fixture.workspace_action("Backend", "startup")
    smoke.wait_for(lambda: fixture.runtime_config()["general"].get("startup_session") == "Backend",
        "named workspace selected for startup")
    after_startup = tomllib.loads(fixture.config.read_text())
    assert after_startup["general"]["startup_session"] == "Backend"
    for section in ("font", "window", "colors"):
        assert after_startup.get(section) == before_startup.get(section), section
    assert fixture.runtime_config()["font"]["size"] == 19.0
    assert abs(fixture.runtime_config()["window"]["opacity"] - 0.72) < 0.001
    subprocess.run([fixture.binary, "msg", "--socket", str(fixture.socket), "config",
        "--window-id", str(fixture.window), "--reset"], env=fixture.env,
        capture_output=True, text=True, check=True, timeout=5)
    smoke.wait_for(lambda: fixture.runtime_config()["font"]["size"] == before_startup["font"]["size"]
        and abs(fixture.runtime_config()["window"]["opacity"] - before_startup["window"]["opacity"]) < 0.001,
        "startup selection preserved appearance overrides without persisting them")
    readonly_startup_workspace(fixture)
    fixture.refresh_size()
    fixture.x11.chord("Control_L", "Shift_L", "p")
    fixture.capture("workspaces")
    fixture.x11.chord("Escape")
    localized_panels(fixture)
    fixture.stop()
    fixture.start()
    assert_titles(fixture.snapshot(), ["Alpha", "Beta", "Gamma"])
    assert marker.read_text().splitlines() == ["once"]
    print("ok: duplicate/reopen preserve cwd with fresh shells; move and named workspace lifecycle", flush=True)


def recovery(fixture):
    manual = fixture.state_directory / "tabs.json"
    recovery_file = fixture.state_directory / "recovery.json"
    before = manual.read_bytes()
    assert not recovery_file.exists(), "Recovery must be opt-in"
    fixture.preferences("sessions")
    fixture.tabs(4)
    fixture.x11.chord("space")
    fixture.tabs(3)
    fixture.x11.chord("Return")
    smoke.wait_for(lambda: fixture.runtime_config()["general"]["session_autosave"],
        "automatic recovery enabled through startup preferences")
    fixture.x11.chord("Escape")
    fixture.general(startup_session=None)
    fixture.x11.chord("Control_L", "Shift_L", "t")
    fixture.rename("Recovery")
    marker = fixture.work / "recovery-count"
    fixture.command(f"printf 'once\\n' >> {shlex.quote(str(marker))}")
    smoke.wait_for(marker.exists, "recovery command executed once")
    fixture.toggle_replay()

    def recovered_layout_saved():
        if not recovery_file.exists():
            return None
        snapshot = json.loads(recovery_file.read_text())
        if len(snapshot["tabs"]) == 4 and snapshot["tabs"][3]["replay_last_command"]:
            return snapshot
        return None

    smoke.wait_for(recovered_layout_saved, "recovery stored all tabs and opt-in flag")
    assert manual.read_bytes() == before, "Autosave replaced the manual session"
    assert recovery_file.stat().st_mode & 0o777 == 0o600
    fixture.stop(crash=True)
    fixture.start()
    assert_titles(fixture.snapshot(), ["Alpha", "Beta", "Gamma", "Recovery"])
    assert marker.read_text().splitlines() == ["once"], "Recovery unexpectedly reran a command"
    fixture.general(restore_session=False)
    fixture.stop(crash=True)
    fixture.start()
    assert len(fixture.snapshot()["tabs"]) == 1, "restore_session=false restored old tabs"
    print("ok: opt-in recovery preserves manual snapshot and never reruns commands; restore can be disabled", flush=True)


def corrupted_workspaces(fixture):
    path = fixture.state_directory / "workspaces.json"
    invalid = b'{"version":999,"workspaces":{}}\n'
    path.write_bytes(invalid)
    fixture.x11.chord("Control_L", "Shift_L", "p")
    fixture.x11.type("Do not overwrite")
    fixture.tabs(1)
    fixture.x11.chord("Return")
    time.sleep(0.3)
    assert path.read_bytes() == invalid, "A malformed workspace file was overwritten"
    fixture.capture("workspaces-invalid")
    fixture.x11.chord("Escape")
    assert fixture.app.poll() is None
    print("ok: unsupported workspace document retained after a failed save", flush=True)


def run(binary, screenshot):
    for executable in ["Xvfb", "import", "convert", binary]:
        if not shutil.which(executable):
            raise RuntimeError(f"Missing executable: {executable}")
    with tempfile.TemporaryDirectory(prefix="alacritty-productivity-") as temporary:
        fixture = Fixture(binary, Path(temporary), screenshot)
        try:
            fixture.start()
            tab_bar(fixture)
            settings_navigation(fixture)
            settings(fixture)
            terminal_settings(fixture)
            shortcuts(fixture)
            tab_workspaces(fixture)
            recovery(fixture)
            corrupted_workspaces(fixture)
            fixture.stop()
        except Exception:
            fixture.log.flush()
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
