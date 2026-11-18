import argparse
from contextlib import contextmanager
import importlib.util
import json
import shutil
from pathlib import Path
import subprocess
import tempfile
import time
import tomllib
import unittest


module = importlib.util.spec_from_file_location(
    "shortcut_list_productivity", Path(__file__).with_name("test-productivity.py"))
productivity = importlib.util.module_from_spec(module)
module.loader.exec_module(productivity)
smoke = productivity.smoke
BINARY = None
SCREENSHOT = None
ACTIONS = (
    "CreateNewTab", "DuplicateTab", "ReopenClosedTab", "SplitVertical", "SplitHorizontal",
    "FocusPaneLeft", "FocusPaneRight", "FocusPaneUp", "FocusPaneDown", "TogglePaneZoom",
    "ClosePane", "OpenWorkspaces", "MoveTabLeft", "MoveTabRight", "OpenSettings",
    "SaveSession", "RenameTab", "CloseTab", "SelectPreviousTab", "SelectNextTab",
)


@contextmanager
def fixture(extra=""):
    with tempfile.TemporaryDirectory(prefix="termust-shortcut-list-") as directory:
        terminal = productivity.Fixture(BINARY, Path(directory), SCREENSHOT)
        try:
            terminal.env["WINIT_X11_SCALE_FACTOR"] = "1.0"
            (terminal.work / "import.toml").write_text("[font]\nsize=14.2\n")
            if extra:
                terminal.config.write_text(terminal.config.read_text() + extra)
            terminal.start()
            yield terminal
        except Exception:
            terminal.log.flush()
            print((terminal.work / "app.log").read_text()[-4000:], flush=True)
            raise
        finally:
            terminal.close()
            terminal.xvfb.stdout.close()


def resize(terminal, width, height):
    terminal.x11.lib.XResizeWindow(terminal.x11.display, terminal.window, width, height)
    terminal.x11.lib.XFlush(terminal.x11.display)
    time.sleep(.2)
    terminal.refresh_size()


def search(terminal, text):
    terminal.x11.chord("Control_L", "f")
    terminal.x11.chord("Control_L", "a")
    terminal.x11.chord("BackSpace")
    if text:
        terminal.x11.type(text)
    time.sleep(.1)


def body_geometry(terminal):
    columns, lines, _, _ = terminal.dimensions()
    width = min(columns - 4, 104)
    height = min(30, lines - (2 if lines >= 22 else 0))
    left, top = (columns - width) // 2, (lines - height) // 2
    wide = columns >= 90 and lines >= 22
    control_height = 2 if lines >= 26 else 1
    body_left = left + 2 + (23 if wide else 0)
    body_top = top + (4 if wide else 8)
    body_width = width - 6 - (23 if wide else 0)
    footer_top = top + height - (2 * control_height + 3)
    return body_left, body_top, body_width, footer_top - body_top - 1, control_height, footer_top


def click_result(terminal, shortcut_column=False):
    left, top, width, _, control_height, _ = body_geometry(terminal)
    column = left + (width - 3 if shortcut_column else 3)
    terminal.click(column, top + control_height + 1)


def save(terminal, action, value):
    terminal.x11.chord("Control_L", "Return")
    smoke.wait_for(lambda: terminal.runtime_config()["general"]["shortcuts"].get(action) == value,
        action + " saved from shortcut list")
    terminal.x11.chord("Escape")


class ShortcutListTests(unittest.TestCase):
    def test_all_twenty_actions_can_be_found_edited_saved_and_reloaded(self):
        with fixture() as terminal:
            resize(terminal, 1176, 790)
            original = terminal.config.read_bytes()
            terminal.preferences("shortcuts")
            expected = {}
            for action, key in zip(ACTIONS, "abcdefghijklmnopqrsu"):
                with self.subTest(action=action):
                    search(terminal, action)
                    terminal.x11.chord("Return")
                    terminal.x11.chord("Control_L", "Alt_L", "Shift_L", key)
                    expected[action] = "Ctrl+Alt+Shift+" + key.upper()
                    self.assertEqual(terminal.config.read_bytes(), original,
                        "Shortcut editing saved before explicit Save")
            terminal.capture("shortcut-list-all-actions-draft")
            terminal.x11.chord("Control_L", "Return")
            smoke.wait_for(lambda: all(terminal.runtime_config()["general"]["shortcuts"].get(action)
                == value for action, value in expected.items()), "all twenty shortcut edits saved")
            terminal.x11.chord("Escape")
            saved = tomllib.loads(terminal.config.read_text())
            self.assertEqual(saved["general"]["shortcuts"], expected)
            terminal.stop(crash=True)
            terminal.start()
            self.assertEqual(terminal.runtime_config()["general"]["shortcuts"], expected)
            terminal.x11.chord("Control_L", "Alt_L", "Shift_L", "q")
            terminal.x11.chord("Control_L", "a")
            terminal.x11.type("Rename shortcut works")
            terminal.x11.chord("Return")
            smoke.wait_for(lambda: terminal.x11.title(terminal.window) == "Rename shortcut works",
                "edited RenameTab shortcut opens the rename dialog after restart")

    def test_empty_search_conflict_and_capture_cancel_preserve_the_session(self):
        with fixture() as terminal:
            resize(terminal, 737, 570)
            original = terminal.config.read_bytes()
            initial_pid = terminal.report("shortcut-list-before")[0]
            terminal.preferences("shortcuts")
            search(terminal, "zzzz-no-such-action-zzzz")
            terminal.capture("shortcut-list-no-results")
            terminal.x11.chord("Return")
            terminal.x11.chord("Escape")
            self.assertEqual(terminal.report("shortcut-list-after-no-results")[0], initial_pid)
            self.assertEqual(terminal.config.read_bytes(), original)
            terminal.preferences("shortcuts")
            search(terminal, "RenameTab")
            terminal.x11.chord("Return")
            terminal.x11.chord("Control_L", "Shift_L", "t")
            terminal.capture("shortcut-list-conflict")
            terminal.x11.chord("Escape")
            terminal.x11.chord("Escape")
            self.assertEqual(terminal.config.read_bytes(), original)
            self.assertEqual(len(terminal.snapshot()["tabs"]), 1,
                "Conflicting shortcut created a terminal tab while capturing")
            terminal.preferences("shortcuts")
            search(terminal, "RenameTab")
            terminal.x11.chord("Return")
            terminal.x11.chord("Control_L", "Shift_L", "F9")
            terminal.capture("shortcut-list-cancel-draft")
            self.assertEqual(terminal.config.read_bytes(), original)
            terminal.x11.chord("Escape")
            self.assertEqual(terminal.report("shortcut-list-after-cancel")[0], initial_pid)
            self.assertEqual(terminal.config.read_bytes(), original)

    def test_mouse_can_edit_action_and_key_columns_in_both_languages(self):
        with fixture() as terminal:
            for locale in ("en", "pt-BR"):
                terminal.general(language=locale)
                for width, height in ((1176, 790), (737, 570)):
                    resize(terminal, width, height)
                    for action, key, shortcut_column in (("RenameTab", "F9", False),
                            ("CloseTab", "F10", True)):
                        with self.subTest(locale=locale, width=width, action=action):
                            terminal.preferences("shortcuts")
                            search(terminal, action)
                            click_result(terminal, shortcut_column)
                            terminal.x11.chord("Control_L", "Shift_L", key)
                            value = "Ctrl+Shift+" + key
                            save(terminal, action, value)
                            self.assertEqual(tomllib.loads(terminal.config.read_text())["general"][
                                "shortcuts"][action], value)

    @unittest.skipUnless(shutil.which("xclip"), "xclip is unavailable")
    def test_search_paste_and_editing_a_key_filter_remain_usable(self):
        with fixture() as terminal:
            resize(terminal, 737, 570)
            terminal.preferences("shortcuts")
            search(terminal, "F2")
            terminal.x11.chord("Return")
            terminal.x11.chord("F8")
            terminal.x11.chord("Control_L", "a")
            terminal.x11.type("RenameTab")
            terminal.x11.chord("Return")
            terminal.x11.chord("Control_L", "Shift_L", "F9")
            save(terminal, "RenameTab", "Ctrl+Shift+F9")
            terminal.preferences("shortcuts")
            search(terminal, "")
            subprocess.run(["xclip", "-display", terminal.display, "-selection", "clipboard"],
                input="CloseTab", text=True, check=True, timeout=5,
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            terminal.x11.chord("Control_L", "v")
            terminal.capture("shortcut-list-pasted-search")
            terminal.x11.chord("Return")
            terminal.x11.chord("Control_L", "Shift_L", "F10")
            save(terminal, "CloseTab", "Ctrl+Shift+F10")

    def test_footer_disables_and_restores_shortcuts_without_resetting_appearance(self):
        with fixture() as terminal:
            resize(terminal, 737, 570)
            font_size = terminal.runtime_config()["font"]["size"]
            terminal.preferences("shortcuts")
            search(terminal, "RenameTab")
            terminal.x11.chord("Return")
            terminal.x11.chord("Control_L", "Shift_L", "F9")
            save(terminal, "RenameTab", "Ctrl+Shift+F9")
            terminal.preferences("shortcuts")
            search(terminal, "RenameTab")
            terminal.tabs(2)
            terminal.x11.chord("Return")
            terminal.capture("shortcut-list-disable-draft")
            save(terminal, "RenameTab", "")
            terminal.preferences("shortcuts")
            search(terminal, "RenameTab")
            terminal.tabs(3)
            terminal.x11.chord("Return")
            terminal.capture("shortcut-list-defaults-draft")
            save(terminal, "RenameTab", "F2")
            self.assertEqual(terminal.runtime_config()["font"]["size"], font_size,
                "Shortcut defaults also reset appearance settings")

    def test_list_uses_available_space_and_scrolls_in_both_languages_and_themes(self):
        for theme in ("dark", "light"):
            colors = '\n[colors.primary]\nforeground="#20262e"\nbackground="#f5efe9"\n' if theme == "light" else ""
            with fixture(colors) as terminal:
                for locale in ("en", "pt-BR"):
                    terminal.general(language=locale)
                    for width, height in ((1176, 790), (737, 570)):
                        with self.subTest(theme=theme, locale=locale, width=width):
                            resize(terminal, width, height)
                            original = terminal.config.read_bytes()
                            terminal.preferences("shortcuts")
                            left, top, body_width, body_height, control_height, footer = body_geometry(terminal)
                            first = top + control_height + 1
                            path = terminal.capture(f"shortcut-list-{theme}-{locale}-{width}")
                            visible = sum(any(terminal.text_mask(terminal.pixels(path, left + 1,
                                row, body_width - 2))) for row in range(first, top + body_height))
                            self.assertGreaterEqual(visible, 8 if width > 1000 else 3,
                                "Shortcut page still hides actions behind two-item pagination")
                            footer_before = terminal.pixels(path, left, footer, body_width,
                                2 * control_height)
                            body_before = terminal.pixels(path, left, top, body_width, body_height)
                            for _ in range(4):
                                terminal.click(left + body_width // 2, first + 1, 5)
                            scrolled = terminal.capture(f"shortcut-list-{theme}-{locale}-{width}-scrolled")
                            self.assertNotEqual(body_before,
                                terminal.pixels(scrolled, left, top, body_width, body_height))
                            self.assertEqual(footer_before,
                                terminal.pixels(scrolled, left, footer, body_width, 2 * control_height),
                                "Shortcut scroll moved the Save/Cancel or reset controls")
                            search(terminal, "")
                            restored = terminal.capture(f"shortcut-list-{theme}-{locale}-{width}-cleared")
                            self.assertTrue(any(terminal.text_mask(terminal.pixels(restored,
                                left + 1, first, body_width - 2))))
                            terminal.x11.chord("Escape")
                            self.assertEqual(terminal.config.read_bytes(), original)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--screenshot", type=Path)
    parser.add_argument("--case", action="append", choices=[name for name in dir(ShortcutListTests)
        if name.startswith("test_")])
    args = parser.parse_args()
    if args.binary:
        BINARY = str(args.binary.resolve())
    else:
        metadata = json.loads(subprocess.check_output([
            "cargo", "metadata", "--no-deps", "--format-version", "1"], text=True))
        BINARY = str(Path(metadata["target_directory"]) / "debug/termust")
    SCREENSHOT = args.screenshot
    suite = unittest.TestSuite(ShortcutListTests(name) for name in args.case) if args.case else (
        unittest.defaultTestLoader.loadTestsFromTestCase(ShortcutListTests))
    raise SystemExit(not unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful())
