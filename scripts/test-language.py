import argparse
from collections import Counter
import importlib.util
import json
import os
from pathlib import Path
import select
import shlex
import shutil
import subprocess
import tempfile
import time
import tomllib


spec = importlib.util.spec_from_file_location("tab_smoke", Path(__file__).with_name("test-tabs.py"))
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)


class Fixture:
    def __init__(self, binary, work, screenshot):
        self.binary = binary
        self.work = work
        self.screenshot = Path(screenshot) if screenshot else None
        self.config = work / "alacritty.toml"
        self.socket = work / "ipc.sock"
        self.size = work / "size"
        self.log = (work / "app.log").open("w")
        self.app = None
        self.x11 = None
        self.window = None
        self.xvfb = subprocess.Popen(
            ["Xvfb", "-displayfd", "1", "-screen", "0", "1200x900x24", "-ac"],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        ready, _, _ = select.select([self.xvfb.stdout], [], [], 5)
        if not ready:
            self.xvfb.terminate()
            raise RuntimeError("Xvfb did not start")
        self.display = ":" + self.xvfb.stdout.readline().strip()
        for directory in ["home", "config", "state", "runtime"]:
            (work / directory).mkdir(mode=0o700)
        self.env = dict(os.environ, DISPLAY=self.display, WINIT_UNIX_BACKEND="x11",
            LIBGL_ALWAYS_SOFTWARE="1", HOME=str(work / "home"),
            XDG_CONFIG_HOME=str(work / "config"), XDG_STATE_HOME=str(work / "state"),
            XDG_RUNTIME_DIR=str(work / "runtime"))
        for name in ["WAYLAND_DISPLAY", "BASH_ENV", "ENV", "ZDOTDIR", "ALACRITTY_SOCKET",
            "ALACRITTY_WINDOW_ID", "LC_ALL", "LC_MESSAGES", "LANGUAGE"]:
            self.env.pop(name, None)
        self.x11 = smoke.X11(self.display)
        (work / "import.toml").write_text('[font]\nsize = 12.0\n')
        self.write_config("en")

    def write_config(self, language):
        self.config.write_text(f'''# Language test fixture: retain this comment.
[general]
confirm_tab_close = false
language = "{language}" # Retain the language comment.
import = ["import.toml"]
live_config_reload = true
ipc_socket = true
[env]
LANGUAGE_TEST = "preserved"
[window]
title = "language-ui"
dynamic_title = false
decorations = "None"
dimensions = {{ columns = 100, lines = 32 }}
padding = {{ x = 0, y = 0 }}
dynamic_padding = false
[terminal.shell]
program = "/bin/bash"
args = ["--noprofile", "--norc"]
[colors.footer_bar]
foreground = "#101010"
background = "#dddddd"
''')

    def start(self, options=(), **locale):
        env = dict(self.env, LANG="C")
        env.update(locale)
        self.app = subprocess.Popen([self.binary, "--config-file", str(self.config),
            "--socket", str(self.socket), *options], env=env, stdout=self.log, stderr=self.log)
        self.window = smoke.wait_for(lambda: next((window for window in self.x11.windows()
            if self.x11.title(window) == "language-ui"), None), "language fixture window opened")
        time.sleep(0.4)
        self.x11.lib.XSetInputFocus(self.x11.display, self.window, 2, 0)
        self.x11.lib.XFlush(self.x11.display)
        self.size.unlink(missing_ok=True)
        self.command(f"cd {shlex.quote(str(self.work))}; stty size > {shlex.quote(str(self.size))}")
        smoke.wait_for(self.size.exists, "fixture shell initialized")
        self.runtime_config()

    def runtime_config(self):
        result = subprocess.run([self.binary, "msg", "--socket", str(self.socket),
            "get-config", "--window-id", "-1"], env=self.env,
            capture_output=True, text=True, check=True, timeout=5)
        return json.loads(result.stdout)

    def wait_language(self, language):
        smoke.wait_for(lambda: self.runtime_config()["general"]["language"] == language,
            f"runtime language changed to {language}")

    def command(self, text):
        self.x11.type(text + "\n")

    def dimensions(self):
        rows, columns = map(int, self.size.read_text().split())
        width, height = self.x11.geometry(self.window)
        return columns, rows, width / columns, height / (rows + 1)

    def click(self, column, line, button=1):
        _, _, cell_width, cell_height = self.dimensions()
        self.x11.click((column + 0.5) * cell_width, (line + 1.5) * cell_height, button)

    def menu(self):
        columns, rows, _, _ = self.dimensions()
        self.click(columns - 1, rows - 1, 3)
        return columns - min(50, columns), rows - min(23, rows)

    def language_page(self):
        for _ in range(4):
            self.x11.chord("Tab")
        self.x11.chord("Return")

    def select_language(self, language):
        index = ["auto", "en", "pt-BR"].index(language)
        for _ in range(index + 2):
            self.x11.chord("Tab")
        self.x11.chord("space")

    def save_language(self, language):
        index = ["auto", "en", "pt-BR"].index(language)
        for _ in range(4 - index):
            self.x11.chord("Tab")
        self.x11.chord("Return")

    def settings(self):
        left, top = self.menu()
        self.click(left + 4, top + 18)
        self.language_page()

    def toolbar_settings(self):
        columns, _, cell_width, cell_height = self.dimensions()
        column = min(30, columns - 6) + 4
        self.x11.click((column + 0.5) * cell_width, cell_height / 2)
        for _ in range(12):
            self.x11.chord("Down")
        self.x11.chord("Return")
        self.language_page()

    def capture(self, label):
        time.sleep(0.15)
        path = self.work / f"{label}.png"
        subprocess.run(["import", "-display", self.display, "-window", str(self.window),
            str(path)], check=True, timeout=5)
        if self.screenshot:
            destination = self.screenshot.with_name(
                self.screenshot.stem + "." + label + self.screenshot.suffix)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, destination)
        return path

    def pixels(self, path, column, line, width, height=1):
        _, _, cell_width, cell_height = self.dimensions()
        crop = f"{int(width * cell_width)}x{int(height * cell_height)}+{int(column * cell_width)}+{int((line + 1) * cell_height)}"
        return subprocess.run(["convert", str(path), "-crop", crop, "+repage",
            "-depth", "8", "rgb:-"], capture_output=True, check=True, timeout=5).stdout

    @staticmethod
    def same_text(left, right):
        return len(left) == len(right) and sum(a != b for a, b in zip(left, right)) <= max(2, len(left) // 1000)

    @staticmethod
    def text_mask(pixels):
        colors = list(zip(pixels[0::3], pixels[1::3], pixels[2::3]))
        background = Counter(colors).most_common(1)[0][0]
        distances = [sum(abs(value - base) for value, base in zip(color, background))
            for color in colors]
        threshold = max(distances) / 2
        return bytes(distance > threshold for distance in distances)

    def settings_header(self, label):
        columns, rows, _, _ = self.dimensions()
        width = min(104, columns - 4)
        height = min(30, rows - (2 if rows >= 22 else 0))
        left, top = (columns - width) // 2, (rows - height) // 2
        return self.text_mask(self.pixels(self.capture(label), left + 2, top + 1, 40))

    def menu_header(self, label):
        left, top = self.menu()
        path = self.capture(label)
        pixels = self.pixels(path, left + 1, top + 1, 38)
        self.x11.chord("Escape")
        return self.text_mask(pixels)

    def search_footer(self, label, direction="f"):
        _, rows, _, _ = self.dimensions()
        self.x11.chord("Control_L", "Shift_L", direction)
        self.x11.chord("Control_L", "u")
        self.x11.type("fixture-query")
        pixels = self.pixels(self.capture(label), 0, rows - 1, 5)
        assert pixels.count(bytes.fromhex("dddddd")) > len(pixels) // 6, "Search footer did not open"
        self.x11.chord("Escape")
        return pixels

    def stop(self):
        if self.app is None or self.app.poll() is not None:
            return
        self.x11.chord("Escape")
        confirmation = self.runtime_config()["general"]["confirm_tab_close"]
        self.x11.chord("Control_L", "Shift_L", "w")
        if confirmation:
            self.x11.chord("y")
        smoke.wait_for(lambda: self.app.poll() == 0, "language fixture closed")
        self.app = None

    def close(self):
        if self.app and self.app.poll() is None:
            self.app.terminate()
            self.app.wait(timeout=5)
        if self.x11:
            self.x11.lib.XCloseDisplay(self.x11.display)
        self.xvfb.terminate()
        self.xvfb.wait(timeout=5)
        self.log.close()


def run(binary, screenshot):
    for executable in ["Xvfb", "import", "convert", binary]:
        if not shutil.which(executable):
            raise RuntimeError(f"Missing executable: {executable}")
    with tempfile.TemporaryDirectory(prefix="alacritty-language-") as temporary:
        fixture = Fixture(binary, Path(temporary), screenshot)
        try:
            fixture.start(LC_ALL="pt_BR.UTF-8", LC_MESSAGES="C")
            fixture.wait_language("en")
            english = fixture.menu_header("menu-en")
            search_english = fixture.search_footer("search-en")
            search_backward_english = fixture.search_footer("search-backward-en", "b")
            fixture.settings()
            settings_english = fixture.settings_header("settings-en")
            fixture.x11.chord("Escape")
            fixture.command('echo $$ > original-pid; printf %s "$LC_ALL|$LC_MESSAGES|$LANG" > locale-before')
            smoke.wait_for(lambda: (fixture.work / "original-pid").exists(), "original shell PID recorded")
            original_pid = (fixture.work / "original-pid").read_text()
            app_pid = fixture.app.pid
            original_configuration = fixture.config.read_bytes()
            fixture.settings()
            fixture.select_language("pt-BR")
            assert fixture.settings_header("settings-cancel-draft") != settings_english
            fixture.wait_language("en")
            fixture.x11.chord("Escape")
            assert fixture.config.read_bytes() == original_configuration
            fixture.settings()
            assert fixture.same_text(fixture.settings_header("settings-cancel-restored"), settings_english)
            fixture.x11.chord("Escape")

            fixture.settings()
            fixture.select_language("pt-BR")
            draft_portuguese = fixture.settings_header("settings-draft-pt")
            assert draft_portuguese != settings_english
            background = fixture.runtime_config()["colors"]["primary"]["background"]
            fixture.config.write_text(fixture.config.read_text() + '\n[colors.primary]\nbackground = "#23282d"\n')
            smoke.wait_for(lambda: fixture.runtime_config()["colors"]["primary"]["background"] != background,
                "unrelated theme change reloaded")
            fixture.wait_language("en")
            assert fixture.same_text(fixture.settings_header("settings-draft-after-theme"), draft_portuguese)
            fixture.save_language("pt-BR")
            fixture.wait_language("pt-BR")
            fixture.x11.chord("Escape")
            persisted = tomllib.loads(fixture.config.read_text())
            assert persisted["general"]["language"] == "pt-BR"
            assert persisted["general"]["import"] == ["import.toml"]
            assert persisted["general"]["live_config_reload"] is True
            assert persisted["env"]["LANGUAGE_TEST"] == "preserved"
            assert "# Language test fixture: retain this comment." in fixture.config.read_text()
            assert "# Retain the language comment." in fixture.config.read_text()
            portuguese = fixture.menu_header("menu-pt")
            assert portuguese != english, "Menu header did not change language"
            assert fixture.search_footer("search-pt") != search_english
            assert fixture.search_footer("search-backward-pt", "b") != search_backward_english
            fixture.settings()
            assert fixture.settings_header("settings-pt") != settings_english
            fixture.x11.chord("Escape")
            fixture.x11.chord("Control_L", "Shift_L", "r")
            fixture.capture("tab-pt")
            fixture.x11.chord("Escape")
            fixture.command('echo $$ > current-pid; printf %s "$LC_ALL|$LC_MESSAGES|$LANG" > locale-after')
            smoke.wait_for(lambda: (fixture.work / "current-pid").exists(), "existing shell remains responsive")
            assert (fixture.work / "current-pid").read_text() == original_pid
            assert fixture.app.pid == app_pid
            smoke.wait_for(lambda: (fixture.work / "locale-after").exists(), "PTY locale preserved")
            assert (fixture.work / "locale-after").read_text() == (fixture.work / "locale-before").read_text()
            print("ok: explicit English overrides locale; GUI Portuguese persists without restarting PTY", flush=True)

            unicode_text = "ação, 日本語, русский, café\n"
            (fixture.work / "unicode-input").write_text(unicode_text)
            fixture.command("cat unicode-input; cat unicode-input > unicode-output; printf '%s' \"$LANGUAGE_TEST\" > env-output")
            smoke.wait_for(lambda: (fixture.work / "env-output").exists(), "Unicode terminal fixture executed")
            assert (fixture.work / "unicode-output").read_text() == unicode_text
            assert (fixture.work / "env-output").read_text() == "preserved"
            fixture.capture("terminal-unicode")
            fixture.stop()
            fixture.start(LC_ALL="C")
            fixture.wait_language("pt-BR")
            assert fixture.same_text(fixture.menu_header("menu-pt-restart"), portuguese)
            print("ok: language persists across restart and terminal output stays unchanged", flush=True)

            fixture.settings()
            fixture.write_config("en")
            fixture.wait_language("en")
            assert fixture.same_text(fixture.settings_header("settings-live-en"), settings_english)
            fixture.x11.chord("Escape")
            assert fixture.same_text(fixture.menu_header("menu-live-en"), english)
            fixture.x11.chord("Control_L", "Shift_L", "r")
            fixture.capture("tab-en")
            fixture.x11.chord("Escape")
            print("ok: live configuration changes update open settings and subsequent menus", flush=True)
            fixture.stop()

            fixture.write_config("auto")
            for label, locale, expected in [
                ("auto-pt", {"LC_ALL": "pt_BR.UTF-8", "LC_MESSAGES": "en_US.UTF-8"}, portuguese),
                ("auto-messages-pt", {"LC_MESSAGES": "pt_BR.UTF-8", "LANG": "en_US.UTF-8"}, portuguese),
                ("auto-lang-pt", {"LANG": "pt_BR.UTF-8"}, portuguese),
                ("auto-c", {"LC_ALL": "C", "LC_MESSAGES": "pt_BR.UTF-8"}, english),
                ("auto-unsupported", {"LC_ALL": "fr_FR.UTF-8", "LC_MESSAGES": "pt_BR.UTF-8"}, english),
            ]:
                fixture.start(**locale)
                fixture.wait_language("auto")
                assert fixture.same_text(fixture.menu_header(label), expected), f"Incorrect detected UI language: {label}"
                fixture.stop()
            print("ok: automatic language honors LC_ALL/LC_MESSAGES and falls back to English", flush=True)

            fixture.config = fixture.work / "new-config" / "alacritty.toml"
            options = ["--title", "language-ui", "-o",
                'window.dynamic_title=false', 'window.decorations="None"',
                'window.dimensions={columns=100,lines=32}', 'window.padding={x=0,y=0}',
                'window.dynamic_padding=false', 'font.size=12.0',
                'terminal.shell={program="/bin/bash",args=["--noprofile","--norc"]}']
            fixture.start(options=options, LC_ALL="C")
            assert not fixture.config.exists()
            fixture.wait_language("auto")
            fixture.toolbar_settings()
            fixture.capture("settings-create-config")
            fixture.select_language("pt-BR")
            fixture.save_language("pt-BR")
            fixture.wait_language("pt-BR")
            assert tomllib.loads(fixture.config.read_text())["general"]["language"] == "pt-BR"
            fixture.x11.chord("Escape")
            fixture.size.unlink()
            fixture.command(f"stty size > {shlex.quote(str(fixture.size))}")
            smoke.wait_for(fixture.size.exists, "geometry updated after creating configuration")
            fixture.config.write_text(fixture.config.read_text().replace(
                'language = "pt-BR"', 'language = "en"'))
            fixture.wait_language("en")
            fixture.size.unlink()
            fixture.command(f"stty size > {shlex.quote(str(fixture.size))}")
            smoke.wait_for(fixture.size.exists, "geometry updated after live reload of new configuration")
            assert fixture.same_text(fixture.menu_header("menu-new-config-live-en"), english)
            fixture.stop()
            print("ok: saving to a missing configuration creates it and enables live reload", flush=True)
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
        metadata = json.loads(subprocess.run(["cargo", "metadata", "--no-deps", "--format-version", "1"],
            capture_output=True, text=True, check=True).stdout)
        binary = Path(metadata["target_directory"]) / "debug" / "alacritty"
    run(str(binary), args.screenshot)
