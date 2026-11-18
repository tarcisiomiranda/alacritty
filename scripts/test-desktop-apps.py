import argparse
from contextlib import contextmanager
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib
import unittest


module = importlib.util.spec_from_file_location(
    "productivity", Path(__file__).with_name("test-productivity.py"))
productivity = importlib.util.module_from_spec(module)
module.loader.exec_module(productivity)
smoke = productivity.smoke
BINARY = None
SCREENSHOT = None


@contextmanager
def fixture(locale="en", system_icons=False):
    with tempfile.TemporaryDirectory(prefix="termust-desktop-apps-") as directory:
        terminal = productivity.Fixture(BINARY, Path(directory), SCREENSHOT)
        try:
            terminal.env["WINIT_X11_SCALE_FACTOR"] = "1.0"
            terminal.write_config(locale)
            programs = terminal.work / "bin"
            programs.mkdir()
            terminal.launches = terminal.work / "launches.jsonl"
            for name in ["code", "zed", "nautilus", "dolphin", "xdg-open"]:
                path = programs / name
                path.write_text(f"#!{sys.executable}\n"
                    "import json, os, sys\n"
                    f"with open({str(terminal.launches)!r}, 'a') as output:\n"
                    "    output.write(json.dumps({'app': os.path.basename(sys.argv[0]), "
                    "'args': sys.argv[1:], 'cwd': os.getcwd()}) + '\\n')\n")
                path.chmod(0o755)
            for name in ["stty", "mkdir", "chmod", "date", "mktemp", "rm", "cat", "uname",
                         "tty", "env", "hostname", "sleep", "readlink", "dirname", "basename", "bwrap"]:
                path = shutil.which(name)
                if path:
                    (programs / name).symlink_to(path)
            terminal.env["PATH"] = str(programs)
            terminal.env["XDG_DATA_HOME"] = str(terminal.work / "data")
            terminal.env["XDG_DATA_DIRS"] = str(terminal.work / "system-data") + (":/usr/share" if system_icons else "")
            terminal.start()
            yield terminal
        except Exception:
            terminal.log.flush()
            print((terminal.work / "app.log").read_text()[-3000:], flush=True)
            raise
        finally:
            terminal.close()
            terminal.xvfb.stdout.close()


def records(terminal):
    if not terminal.launches.exists():
        return []
    return [json.loads(line) for line in terminal.launches.read_text().splitlines()]


def change_folder(terminal, folder):
    encoded = "".join(f"\\x{value:02x}" for value in os.fsencode(folder))
    terminal.command(f"cd -- $'{encoded}'")
    smoke.wait_for(lambda: terminal.report("folder-report")[1] == str(folder), "folder changed")


def menu_action(terminal, editor, mouse=False):
    if mouse:
        left, top = terminal.menu()
        terminal.click(left + 4, top + (3 if editor else 4))
    else:
        terminal.menu()
        terminal.x11.chord("Home")
        if not editor:
            terminal.x11.chord("Down")
        terminal.x11.chord("Return")


def choose_apps(terminal):
    terminal.preferences("applications")
    terminal.tabs(3)
    terminal.x11.chord("space")
    terminal.tabs(3)
    terminal.x11.chord("space")


class DesktopApplicationTests(unittest.TestCase):
    def test_named_apps_receive_the_literal_current_folder_without_shell_input(self):
        for locale in ["en", "pt-BR"]:
            with self.subTest(locale=locale), fixture(locale) as terminal:
                folder = terminal.work / "ação $(touch injected); 'quoted folder'"
                folder.mkdir()
                change_folder(terminal, folder)
                shell_pid, _ = terminal.report("original-shell")
                sentinel = terminal.work / "shell-command-ran"
                for app, editor in [("code", True), ("zed", True),
                                    ("nautilus", False), ("dolphin", False)]:
                    terminal.general(**{("editor" if editor else "file_manager"): app})
                    previous = len(records(terminal))
                    terminal.x11.type(f"printf unexpected > {sentinel}")
                    menu_action(terminal, editor, mouse=app in ["code", "nautilus"])
                    smoke.wait_for(lambda: len(records(terminal)) == previous + 1,
                                   f"{app} received the folder")
                    terminal.x11.chord("Control_L", "u")
                    self.assertEqual(records(terminal)[-1],
                                     {"app": app, "args": [str(folder)], "cwd": str(folder)})
                    self.assertFalse(sentinel.exists())
                    self.assertFalse((folder / "injected").exists())
                self.assertEqual(terminal.report("same-shell"), (shell_pid, str(folder)))
                terminal.menu()
                terminal.capture(f"menu-{locale}")
                terminal.x11.chord("Escape")

    def test_preferences_cancel_save_reload_and_restart_in_both_languages(self):
        for locale in ["en", "pt-BR"]:
            with self.subTest(locale=locale), fixture(locale) as terminal:
                choose_apps(terminal)
                self.assertEqual(terminal.runtime_config()["general"]["editor"], "auto")
                self.assertEqual(terminal.runtime_config()["general"]["file_manager"], "auto")
                terminal.x11.chord("Escape")
                self.assertNotIn("editor", tomllib.loads(terminal.config.read_text())["general"])
                choose_apps(terminal)
                terminal.x11.chord("Control_L", "Return")
                smoke.wait_for(lambda: terminal.runtime_config()["general"]["editor"] == "zed"
                    and terminal.runtime_config()["general"]["file_manager"] == "dolphin",
                    "application preferences saved and applied")
                saved = tomllib.loads(terminal.config.read_text())
                self.assertEqual(saved["general"]["editor"], "zed")
                self.assertEqual(saved["general"]["file_manager"], "dolphin")
                self.assertEqual(saved["general"]["import"], ["import.toml"])
                self.assertEqual(saved["env"]["LANGUAGE_TEST"], "preserved")
                terminal.x11.chord("Escape")
                terminal.preferences("applications")
                terminal.capture(f"settings-{locale}")
                terminal.x11.chord("Escape")
                terminal.stop(crash=True)
                terminal.start()
                self.assertEqual(terminal.runtime_config()["general"]["editor"], "zed")
                self.assertEqual(terminal.runtime_config()["general"]["file_manager"], "dolphin")
                menu_action(terminal, True)
                smoke.wait_for(lambda: records(terminal), "saved editor reopened after restart")
                self.assertEqual(records(terminal)[-1]["app"], "zed")

    def test_clicked_split_and_inactive_tab_use_their_own_working_directories(self):
        with fixture() as terminal:
            first = terminal.work / "first folder"
            second = terminal.work / "second folder"
            first.mkdir()
            second.mkdir()
            change_folder(terminal, first)
            _, _, cell_width, cell_height = terminal.dimensions()
            terminal.general(editor="code", file_manager="dolphin")
            terminal.x11.chord("Control_L", "Shift_L", "e")
            time.sleep(.3)
            change_folder(terminal, second)
            terminal.x11.click(10 * cell_width, 12 * cell_height, 3)
            terminal.x11.chord("Home")
            terminal.x11.chord("Down")
            terminal.x11.chord("Return")
            smoke.wait_for(lambda: len(records(terminal)) == 1, "clicked pane launched its folder")
            self.assertEqual(records(terminal)[-1]["args"], [str(first)])
            terminal.x11.chord("Control_L", "Shift_L", "t")
            time.sleep(.3)
            change_folder(terminal, second)
            terminal.x11.click(5 * cell_width, cell_height / 2, 3)
            terminal.x11.chord("Home")
            terminal.x11.chord("Return")
            smoke.wait_for(lambda: len(records(terminal)) == 2, "inactive tab launched its folder")
            self.assertEqual(records(terminal)[-1]["args"], [str(first)])

    def test_app_icons_draw_color_in_menu_gutters_in_both_languages(self):
        for locale in ["en", "pt-BR"]:
            with self.subTest(locale=locale), fixture(locale, system_icons=True) as terminal:
                terminal.general(editor="code", file_manager="nautilus")
                left, top = terminal.menu()
                time.sleep(.25)
                path = terminal.capture(f"app-icons-{locale}")
                for row in [3, 4]:
                    pixels = terminal.pixels(path, left + 1, top + row, 2)
                    colorful = sum(max(color) - min(color) > 40
                        for color in zip(pixels[0::3], pixels[1::3], pixels[2::3]))
                    self.assertGreater(colorful, 12)
                    colors = list(zip(pixels[0::3], pixels[1::3], pixels[2::3]))
                    if row == 3 and Path('/usr/share/pixmaps/vscode.png').is_file():
                        self.assertGreater(sum(red < 30 and blue > 100 for red, green, blue in colors), 8)
                    if row == 4 and Path('/usr/share/icons/hicolor/scalable/apps/org.gnome.Nautilus.svg').is_file():
                        self.assertGreater(sum(min(color) > 190 for color in colors), 2)
                terminal.x11.chord("Escape")
                terminal.command("printf icon-menu-ok > icon-menu-ok")
                smoke.wait_for((terminal.work / "icon-menu-ok").exists, "shell remained usable")

    def test_automatic_preferences_launch_the_editor_and_system_file_manager(self):
        with fixture() as terminal:
            menu_action(terminal, True)
            smoke.wait_for(lambda: len(records(terminal)) == 1, "automatic editor opened")
            menu_action(terminal, False)
            smoke.wait_for(lambda: len(records(terminal)) == 2, "system file manager opened")
            self.assertEqual([record["app"] for record in records(terminal)], ["code", "xdg-open"])
            self.assertTrue(all(record["args"] == [str(terminal.work)] for record in records(terminal)))

    def test_registered_desktop_launcher_opens_the_folder_through_real_gio(self):
        gio = shutil.which("gio")
        self.assertIsNotNone(gio, "GIO is required to verify registered desktop launchers")
        with fixture() as terminal:
            programs = terminal.work / "bin"
            (programs / "zed").rename(programs / "mock-zed")
            (programs / "gio").symlink_to(gio)
            desktop = terminal.work / "data/applications/dev.zed.Zed.desktop"
            desktop.parent.mkdir(parents=True)
            desktop.write_text(f"""[Desktop Entry]
Type=Application
Name=Zed
Exec={programs / 'mock-zed'} %F
Terminal=false
""")
            terminal.general(editor="zed")
            menu_action(terminal, True)
            smoke.wait_for(lambda: len(records(terminal)) == 1, "registered Zed launcher opened")
            self.assertEqual(records(terminal)[0]["app"], "mock-zed")
            self.assertEqual(records(terminal)[0]["args"], [str(terminal.work)])

    def test_application_choices_work_in_narrow_windows_in_both_languages(self):
        for locale in ["en", "pt-BR"]:
            with self.subTest(locale=locale), fixture(locale) as terminal:
                for width in [737, 594]:
                    terminal.x11.lib.XResizeWindow(terminal.x11.display, terminal.window, width, 570)
                    terminal.x11.lib.XFlush(terminal.x11.display)
                    time.sleep(.2)
                    terminal.refresh_size()
                    original = terminal.config.read_bytes()
                    choose_apps(terminal)
                    terminal.capture(f"compact-{locale}-{width}")
                    terminal.x11.chord("Escape")
                    self.assertEqual(terminal.config.read_bytes(), original)
                    self.assertEqual(terminal.runtime_config()["general"]["editor"], "auto")
                    self.assertEqual(terminal.runtime_config()["general"]["file_manager"], "auto")

    def test_missing_app_or_deleted_folder_never_launches_a_fallback(self):
        with fixture("pt-BR") as terminal:
            terminal.general(editor="zed")
            (terminal.work / "bin" / "zed").unlink()
            menu_action(terminal, True)
            time.sleep(.3)
            self.assertFalse(records(terminal))
            terminal.capture("missing-app-pt-BR")
            terminal.x11.chord("Escape")
            folder = terminal.work / "deleted-folder"
            folder.mkdir()
            change_folder(terminal, folder)
            folder.rmdir()
            menu_action(terminal, False)
            time.sleep(.3)
            self.assertFalse(records(terminal))
            self.assertIsNone(terminal.app.poll())


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--screenshot", type=Path)
    parser.add_argument("--case", action="append", choices=[name for name in dir(DesktopApplicationTests)
                        if name.startswith("test_")])
    args = parser.parse_args()
    if args.binary:
        BINARY = str(args.binary.resolve())
    else:
        metadata = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"], text=True))
        BINARY = str(Path(metadata["target_directory"]) / "debug/termust")
    SCREENSHOT = args.screenshot
    suite = unittest.TestSuite(DesktopApplicationTests(name) for name in args.case) if args.case else (
        unittest.defaultTestLoader.loadTestsFromTestCase(DesktopApplicationTests))
    raise SystemExit(not unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful())
