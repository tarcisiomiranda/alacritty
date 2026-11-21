import argparse
from collections import Counter
from contextlib import contextmanager
import importlib.util
import json
from pathlib import Path
import subprocess
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
def fixture():
    with tempfile.TemporaryDirectory(prefix="termust-compact-settings-") as directory:
        terminal = productivity.Fixture(BINARY, Path(directory), SCREENSHOT)
        try:
            terminal.env["WINIT_X11_SCALE_FACTOR"] = "1.0"
            (terminal.work / "import.toml").write_text("[font]\nsize=14.2\n")
            terminal.start()
            yield terminal
        except Exception:
            terminal.log.flush()
            print((terminal.work / "app.log").read_text()[-3000:], flush=True)
            raise
        finally:
            terminal.close()
            terminal.xvfb.stdout.close()


def resize(terminal, width, height):
    terminal.x11.lib.XResizeWindow(terminal.x11.display, terminal.window, width, height)
    terminal.x11.lib.XFlush(terminal.x11.display)
    time.sleep(.2)
    terminal.refresh_size()


def assert_bottom_padding(test, terminal, path):
    columns, rows, cell_width, _ = terminal.dimensions()
    pixels = terminal.pixels(path, 4, rows - 1, columns - 8)
    frame = 5 * int((columns - 8) * cell_width) * 3
    pixels = pixels[:-frame]
    colors = list(zip(pixels[0::3], pixels[1::3], pixels[2::3]))
    background = Counter(colors).most_common(1)[0][0]
    foreground = sum(sum(abs(value - base) for value, base in zip(color, background)) > 240
        for color in colors)
    test.assertLess(foreground, len(colors) // 100,
        "Footer text reaches the bottom cell instead of leaving safe padding")


class CompactSettingsTests(unittest.TestCase):
    def test_mouse_preview_cancel_and_save_at_user_window_sizes(self):
        for width in [737, 594]:
            with self.subTest(width=width), fixture() as terminal:
                resize(terminal, width, 570)
                columns, rows, _, _ = terminal.dimensions()
                self.assertEqual(rows, 20)
                original = terminal.config.read_bytes()
                terminal.preferences()
                terminal.capture(f"mouse-{columns}x{rows}")
                terminal.click(columns - 20, 11)
                smoke.wait_for(lambda: abs(terminal.runtime_config()["window"]["opacity"] - .95)
                    < .001, "compact opacity stepper previews its value")
                self.assertEqual(terminal.config.read_bytes(), original)
                terminal.click(columns - 9, rows - 4)
                smoke.wait_for(lambda: terminal.runtime_config()["window"]["opacity"] == 1,
                    "compact Cancel restores the preview")
                self.assertEqual(terminal.config.read_bytes(), original)
                terminal.preferences()
                terminal.click(columns - 20, 11)
                terminal.click(columns - 21, rows - 4)
                smoke.wait_for(lambda: abs(tomllib.loads(terminal.config.read_text()).get(
                    "window", {}).get("opacity", 1) - .95) < .001,
                    "compact Save persists the selected value")

    def test_all_pages_keep_footer_inside_normal_compact_and_gnome_viewports(self):
        with fixture() as terminal:
            for theme in ["dark", "light"]:
                if theme == "light":
                    terminal.config.write_text(terminal.config.read_text() +
                        '\n[colors.primary]\nforeground="#20262e"\nbackground="#f5efe9"\n')
                    smoke.wait_for(lambda: terminal.runtime_config()["colors"]["primary"][
                        "background"] == "#f5efe9", "light palette reloaded")
                for locale in ["en", "pt-BR"]:
                    terminal.general(language=locale)
                    for width, height in [(737, 570), (594, 570), (495, 513), (1100, 864)]:
                        resize(terminal, width, height)
                        columns, rows, _, _ = terminal.dimensions()
                        original = terminal.config.read_bytes()
                        for page in ["appearance", "terminal", "tabs", "sessions", "language", "shortcuts"]:
                            terminal.preferences(page)
                            label = f"{theme}-{locale}-{columns}x{rows}-{page}"
                            path = terminal.capture(label)
                            assert_bottom_padding(self, terminal, path)
                            terminal.x11.chord("Escape")
                        self.assertEqual(terminal.config.read_bytes(), original)


    def test_compact_escape_hint_matches_the_rendered_key_in_both_languages(self):
        with fixture() as terminal:
            terminal.rename("Termust")
            for width, locale in [(737, "en"), (594, "en"), (594, "pt-BR")]:
                resize(terminal, width, 570)
                terminal.general(language=locale)
                terminal.command("printf '\\033[2J\\033[HEsc'")
                time.sleep(.15)
                reference = terminal.capture("escape-reference")
                columns, rows, _, _ = terminal.dimensions()
                width, height = terminal.x11.geometry(terminal.window)
                cell_width, cell_height = width // columns, height // (rows + 1)

                def pixels(path, column, line):
                    return subprocess.check_output(["magick", str(path), "-crop",
                        f"{3 * cell_width}x{cell_height}+{column * cell_width}+{(line + 1) * cell_height}",
                        "+repage", "-depth", "8", "rgb:-"])

                expected = terminal.text_mask(pixels(reference, 0, 0))
                terminal.command("PS1=; printf '\\033[2J\\033[H'")
                time.sleep(.15)
                terminal.preferences()
                path = terminal.capture(f"appearance-{locale}-{columns}x{rows}")
                assert_bottom_padding(self, terminal, path)
                if columns < 64:
                    prefix = "Tab: next  Enter: OK  " if locale == "en" else "Tab: próximo  Enter: OK  "
                    actual = terminal.text_mask(pixels(path, 4 + len(prefix), rows - 2))
                    self.assertTrue(terminal.same_text(expected, actual),
                        f"Esc glyph missing from the {locale} compact footer")
                terminal.x11.chord("Escape")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--screenshot", type=Path)
    parser.add_argument("--case", action="append", choices=[name for name in dir(CompactSettingsTests)
        if name.startswith("test_")])
    args = parser.parse_args()
    if args.binary:
        BINARY = str(args.binary.resolve())
    else:
        metadata = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"], text=True))
        BINARY = str(Path(metadata["target_directory"]) / "debug/termust")
    SCREENSHOT = args.screenshot
    suite = unittest.TestSuite(CompactSettingsTests(name) for name in args.case) if args.case else (
        unittest.defaultTestLoader.loadTestsFromTestCase(CompactSettingsTests))
    raise SystemExit(not unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful())
