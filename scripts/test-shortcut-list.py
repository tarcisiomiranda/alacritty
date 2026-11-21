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
    "shortcut_list_ui", Path(__file__).with_name("test-ui-font.py"))
ui = importlib.util.module_from_spec(module)
module.loader.exec_module(ui)
productivity = ui.productivity
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


def rendered_controls(terminal, frame):
    width, _ = terminal.x11.geometry(terminal.window)
    colors = terminal.runtime_config()["colors"]
    background = ui.rgb(colors["primary"]["background"])
    foreground = ui.rgb(colors["primary"]["foreground"])
    accent = ui.rgb(colors["normal"]["blue"])
    panel = tuple(round(a * .965 + b * .035) for a, b in zip(background, foreground))
    selected = tuple(round(a * .84 + b * .16) for a, b in zip(panel, accent))
    return ui.largest_region(frame, width, selected), ui.largest_region(frame, width, accent)


def crop(frame, width, bounds):
    left, top, right, bottom = bounds
    return b"".join(frame[(y * width + left) * 3:(y * width + right + 1) * 3]
        for y in range(top, bottom + 1))


def click_result(terminal, shortcut_column=False):
    frame = ui.pixels(terminal.capture("shortcut-target"))
    (left, top, right, bottom), save_button = rendered_controls(terminal, frame)
    x = left + (right - left) * (.85 if shortcut_column else .2)
    terminal.x11.click(x, (top + bottom) / 2)
    return save_button


def click_button(terminal, bounds):
    left, top, right, bottom = bounds
    terminal.x11.click((left + right) / 2, (top + bottom) / 2)


def save(terminal, action, value, button=None):
    if button is None:
        terminal.x11.chord("Control_L", "Return")
    else:
        click_button(terminal, button)
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

    def test_split_shortcuts_swap_only_after_confirmation(self):
        with fixture() as terminal:
            resize(terminal, 1176, 790)
            terminal.general(language="pt-BR")
            original = terminal.config.read_bytes()
            terminal.preferences("shortcuts")
            search(terminal, "dividir vertical")
            button = click_result(terminal)
            terminal.x11.chord("Control_L", "Shift_L", "j")
            click_button(terminal, button)
            self.assertEqual(terminal.config.read_bytes(), original)
            self.assertEqual(terminal.runtime_config()["general"]["shortcuts"], {})
            terminal.x11.chord("Escape")
            terminal.x11.chord("Return")
            terminal.x11.chord("Control_L", "Shift_L", "j")
            terminal.x11.chord("Return")
            self.assertEqual(terminal.config.read_bytes(), original)
            save(terminal, "SplitVertical", "Ctrl+Shift+J", button)
            self.assertEqual(tomllib.loads(terminal.config.read_text())["general"]["shortcuts"],
                {"SplitVertical": "Ctrl+Shift+J", "SplitHorizontal": "Ctrl+Shift+E"})
            terminal.x11.chord("Control_L", "Shift_L", "j")
            terminal.x11.chord("Control_L", "Shift_L", "e")
            self.assert_split_layout(terminal, "vertical", "horizontal")

    def test_split_bindings_apply_after_successful_mouse_save_and_restart(self):
        with fixture() as terminal:
            resize(terminal, 737, 570)
            terminal.preferences("shortcuts")
            for action, keys in (("split vertical", ("Control_L", "KP_Enter")),
                    ("split horizontal", ("Control_L", "Shift_L", "F9"))):
                search(terminal, action)
                button = click_result(terminal, shortcut_column=True)
                terminal.x11.chord(*keys)
            original = terminal.config.read_bytes()
            initial = ui.pixels(terminal.capture("splits-before-save"))
            width, _ = terminal.x11.geometry(terminal.window)
            terminal.config.chmod(0o444)
            click_button(terminal, button)
            time.sleep(.4)
            failed = ui.pixels(terminal.capture("splits-save-failed"))
            self.assertEqual(original, terminal.config.read_bytes())
            self.assertEqual(terminal.runtime_config()["general"]["shortcuts"], {})
            self.assertNotEqual(crop(initial, width, button), crop(failed, width, button))
            terminal.config.chmod(0o600)
            left, top, _, bottom = button
            sample = f"1x1+{left + 6}+{(top + bottom) // 2}"
            def fill():
                return subprocess.check_output(["import", "-display", terminal.display, "-window",
                    str(terminal.window), "-crop", sample, "-depth", "8", "rgb:-"], timeout=5)
            accent = fill()
            click_button(terminal, button)
            smoke.wait_for(lambda: fill() != accent, "Save button animates on click")
            smoke.wait_for(lambda: fill() == accent, "Save button settles")
            saved = ui.pixels(terminal.capture("splits-saved"))
            self.assertNotEqual(crop(failed, width, button), crop(saved, width, button))
            self.assertNotEqual(crop(initial, width, button), crop(saved, width, button))
            expected = {"SplitVertical": "Ctrl+NumpadEnter", "SplitHorizontal": "Ctrl+Shift+F9"}
            self.assertEqual(terminal.runtime_config()["general"]["shortcuts"], expected)
            self.assertEqual(tomllib.loads(terminal.config.read_text())["general"]["shortcuts"], expected)
            terminal.x11.chord("Escape")
            for restarted in (False, True):
                if restarted:
                    terminal.stop(crash=True)
                    terminal.start()
                    terminal.x11.chord("Control_L", "Shift_L", "t")
                terminal.x11.chord("Control_L", "Shift_L", "e")
                terminal.x11.chord("Control_L", "Shift_L", "j")
                terminal.x11.chord("Control_L", "Return")
                terminal.x11.chord("Control_L", "Shift_L", "F9")
                terminal.x11.chord("Control_L", "KP_Enter")
                self.assert_split_layout(terminal, "horizontal", "vertical")

    def assert_split_layout(self, terminal, outer, inner):
        session = terminal.snapshot()
        tab = session["tabs"][session["active"]]
        self.assertEqual(len(tab.get("panes", [])), 2)
        split = tab["layout"]["split"]
        self.assertEqual(split["axis"], outer)
        self.assertEqual(split["second"]["split"]["axis"], inner)

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
                            button = click_result(terminal, shortcut_column)
                            modifiers = ("Control_L",) if width > 1000 else ("Control_L", "Shift_L")
                            key = key if locale == "en" else ("F7" if key == "F9" else "F8")
                            terminal.x11.chord(*modifiers, key)
                            value = ("Ctrl+" if len(modifiers) == 1 else "Ctrl+Shift+") + key
                            save(terminal, action, value, button)
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
                            frame = ui.pixels(terminal.capture(f"shortcut-list-{theme}-{locale}-{width}"))
                            row, button = rendered_controls(terminal, frame)
                            left, first, right, bottom = row
                            row_height = bottom - first + 1
                            footer_top = button[1] - (button[3] - button[1] + 5)
                            body = (left + 10, first, right - 10, footer_top - 16)
                            footer = (0, footer_top, width - 1, button[3] + 1)
                            visible = sum(any(terminal.text_mask(crop(frame, width,
                                (left + 10, y, right - 10, y + row_height - 1))))
                                for y in range(first, body[3] - row_height + 2, row_height))
                            self.assertGreaterEqual(visible, 8 if width > 1000 else 3)
                            for _ in range(4):
                                terminal.x11.click((left + right) / 2, first + row_height, 5)
                            scrolled = ui.pixels(terminal.capture(f"shortcut-list-{theme}-{locale}-{width}-scrolled"))
                            self.assertNotEqual(crop(frame, width, body), crop(scrolled, width, body))
                            self.assertEqual(crop(frame, width, footer), crop(scrolled, width, footer))
                            search(terminal, "")
                            restored = ui.pixels(terminal.capture(f"shortcut-list-{theme}-{locale}-{width}-cleared"))
                            self.assertEqual(crop(frame, width, row), crop(restored, width, row))
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
