import argparse
import hashlib
import importlib.util
import json
import subprocess
import tempfile
import time
import tomllib
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("productivity", ROOT / "scripts/test-productivity.py")
productivity = importlib.util.module_from_spec(spec)
spec.loader.exec_module(productivity)
BINARY = None
SCREENSHOT = None


def native_icon(terminal):
    output = subprocess.check_output(["xprop", "-id", str(terminal.window),
        "-f", "_NET_WM_ICON", "32c", "_NET_WM_ICON"], env=terminal.env, timeout=5)
    return hashlib.sha256(output).hexdigest()


def open_icons(terminal):
    terminal.preferences()
    terminal.tabs(8)
    terminal.x11.chord("Return")
    time.sleep(.15)


def wait_icon(terminal, preset):
    productivity.smoke.wait_for(lambda: terminal.runtime_config()["window"]["icon"] == preset,
        f"icon preview changed to {preset}")


def click_cat(terminal, screenshot="picker"):
    path = terminal.capture(screenshot)
    pixels = subprocess.check_output(["magick", str(path), "-depth", "8", "rgb:-"], timeout=10)
    width, _ = terminal.x11.geometry(terminal.window)
    points = [offset // 3 for offset in range(0, len(pixels), 3)
        if pixels[offset:offset + 3] == bytes.fromhex("e9dcff")]
    if len(points) < 20:
        raise AssertionError(f"the cat artwork was not visible in {screenshot}")
    terminal.x11.click(sum(point % width for point in points) / len(points),
        sum(point // width for point in points) / len(points))


class IconPersonalizationTests(unittest.TestCase):
    def save_icon(self, terminal, launcher, preset):
        terminal.x11.chord("Control_L", "Return")
        marker = f"X-Termust-IconPreset={preset}\n"
        productivity.smoke.wait_for(
            lambda: tomllib.loads(terminal.config.read_text())["window"].get("icon") == preset
            and marker in launcher.read_text(),
            f"{preset} icon saved",
        )
        text = launcher.read_text()
        self.assertIn("Name=My terminal\n", text)
        icon_path = next(line[5:] for line in text.splitlines() if line.startswith("Icon="))
        self.assertEqual(Path(icon_path).read_bytes(), (ROOT / f"extra/icons/{preset}.png").read_bytes())

    def test_preview_cancel_save_restart_and_restore_with_mouse_and_keyboard(self):
        for locale, width, height, scale in [("en", 1100, 800, 1), ("pt-BR", 737, 570, 1), ("pt-BR", 1100, 800, 2)]:
            with self.subTest(locale=locale, scale=scale), tempfile.TemporaryDirectory(prefix="termust-icons-") as directory:
                terminal = productivity.Fixture(BINARY, Path(directory), SCREENSHOT)
                try:
                    terminal.env["WINIT_X11_SCALE_FACTOR"] = str(scale)
                    terminal.write_config(locale)
                    launcher = Path(terminal.env["XDG_DATA_HOME"]) / "applications/Termust.desktop"
                    launcher.parent.mkdir(parents=True)
                    original_launcher = (ROOT / "extra/linux/Termust.desktop").read_text().replace(
                        "Icon=Termust", "Icon=my-original-theme").replace("Name=Termust", "Name=My terminal")
                    launcher.write_text(original_launcher)
                    terminal.start()
                    terminal.x11.lib.XResizeWindow(terminal.x11.display, terminal.window, width, height)
                    terminal.x11.lib.XFlush(terminal.x11.display)
                    subprocess.run([BINARY, "msg", "--socket", str(terminal.socket), "config",
                        "--window-id", str(terminal.window), "font.size=22"], env=terminal.env, check=True, timeout=5)
                    original_config = terminal.config.read_bytes()
                    original_icon = native_icon(terminal)
                    open_icons(terminal)
                    seen = {original_icon}
                    for preset in ("cat", "dog", "fox", "robot", "yorkie"):
                        terminal.tabs(1)
                        terminal.x11.chord("Return")
                        wait_icon(terminal, preset)
                        current = native_icon(terminal)
                        self.assertNotIn(current, seen)
                        seen.add(current)
                        terminal.capture(f"{locale}-dpi{scale}-{preset}-preview")
                    self.assertEqual(terminal.config.read_bytes(), original_config)
                    self.assertEqual(launcher.read_text(), original_launcher)
                    terminal.x11.chord("Escape")
                    wait_icon(terminal, "termust")
                    self.assertEqual(native_icon(terminal), original_icon)
                    open_icons(terminal)
                    click_cat(terminal)
                    wait_icon(terminal, "cat")
                    self.save_icon(terminal, launcher, "cat")
                    cat_icon = native_icon(terminal)
                    terminal.x11.chord("Escape")
                    self.assertEqual(native_icon(terminal), cat_icon)
                    terminal.stop()
                    terminal.start()
                    terminal.x11.lib.XResizeWindow(terminal.x11.display, terminal.window, width, height)
                    terminal.x11.lib.XFlush(terminal.x11.display)
                    wait_icon(terminal, "cat")
                    self.assertEqual(native_icon(terminal), cat_icon)
                    terminal.preferences()
                    click_cat(terminal, f"appearance-preview-{locale}-dpi{scale}")
                    terminal.tabs(1)
                    terminal.x11.chord("Return")
                    wait_icon(terminal, "dog")
                    terminal.x11.chord("Escape")
                    wait_icon(terminal, "cat")
                    self.assertEqual(native_icon(terminal), cat_icon)
                    open_icons(terminal)
                    terminal.tabs(4)
                    terminal.x11.chord("Return")
                    wait_icon(terminal, "yorkie")
                    self.save_icon(terminal, launcher, "yorkie")
                    yorkie_icon = native_icon(terminal)
                    terminal.stop()
                    terminal.start()
                    wait_icon(terminal, "yorkie")
                    self.assertEqual(native_icon(terminal), yorkie_icon)
                    open_icons(terminal)
                    terminal.tabs(1)
                    terminal.x11.chord("Return")
                    wait_icon(terminal, "termust")
                    terminal.x11.chord("Control_L", "Return")
                    productivity.smoke.wait_for(lambda: tomllib.loads(terminal.config.read_text())["window"].get("icon") == "termust", "original icon saved")
                    self.assertIn("Icon=my-original-theme\n", launcher.read_text())
                    self.assertNotIn("X-Termust-", launcher.read_text())
                    self.assertEqual(native_icon(terminal), original_icon)
                finally:
                    terminal.close()
                    terminal.xvfb.stdout.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--screenshot", type=Path)
    args = parser.parse_args()
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], text=True, cwd=ROOT))
    BINARY = str(args.binary.resolve() if args.binary else Path(metadata["target_directory"]) / "debug/termust")
    SCREENSHOT = args.screenshot
    unittest.main(argv=[__file__], verbosity=2)
