import argparse
from collections import deque
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import time
import tomllib
import unittest


ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("productivity", ROOT / "scripts/test-productivity.py")
productivity = importlib.util.module_from_spec(spec)
spec.loader.exec_module(productivity)
BINARY = None
SCREENSHOT = None


def rgb(value):
    return tuple(bytes.fromhex(value.lstrip("#")))


def largest_region(pixels, width, color):
    matches = {offset // 3 for offset in range(0, len(pixels), 3) if tuple(pixels[offset:offset + 3]) == color}
    largest = set()
    while matches:
        start = matches.pop()
        group = {start}
        queue = deque([start])
        while queue:
            point = queue.popleft()
            neighbors = [point - width, point + width]
            if point % width:
                neighbors.append(point - 1)
            if point % width < width - 1:
                neighbors.append(point + 1)
            for neighbor in neighbors:
                if neighbor in matches:
                    matches.remove(neighbor)
                    group.add(neighbor)
                    queue.append(neighbor)
        if len(group) > len(largest):
            largest = group
    if not largest:
        raise AssertionError(f"color {color} was not rendered")
    return min(point % width for point in largest), min(point // width for point in largest), max(point % width for point in largest), max(point // width for point in largest)


def pixels(path):
    return subprocess.check_output(["convert", str(path), "-depth", "8", "rgb:-"], timeout=10)


class InterfaceFontTests(unittest.TestCase):
    def test_settings_geometry_text_preview_and_mouse_save_stay_stable(self):
        for width, height, scale in [(737, 570, 1), (1100, 800, 1), (1100, 800, 2)]:
            with self.subTest(size=(width, height), dpi=scale), tempfile.TemporaryDirectory(prefix="termust-ui-font-") as directory:
                terminal = productivity.Fixture(BINARY, Path(directory), SCREENSHOT)
                try:
                    terminal.env["WINIT_X11_SCALE_FACTOR"] = str(scale)
                    terminal.start()
                    terminal.x11.lib.XResizeWindow(terminal.x11.display, terminal.window, width, height)
                    terminal.x11.lib.XFlush(terminal.x11.display)
                    time.sleep(.2)
                    baseline = None
                    bounds = None
                    save = None
                    config = terminal.runtime_config()
                    bg = rgb(config["colors"]["primary"]["background"])
                    fg = rgb(config["colors"]["primary"]["foreground"])
                    panel = tuple(int(a * .965 + b * .035 + .5) for a, b in zip(bg, fg))
                    accent = rgb(config["colors"]["normal"]["blue"])
                    for size in [11., 22., 36.]:
                        subprocess.run([BINARY, "msg", "--socket", str(terminal.socket), "config", "--window-id", str(terminal.window), f"font.size={size}"], env=terminal.env, check=True, timeout=5)
                        productivity.smoke.wait_for(lambda: terminal.runtime_config()["font"]["size"] == size, "terminal font changed")
                        if baseline is None:
                            terminal.preferences()
                        path = terminal.capture(f"ui-{width}x{height}-dpi{scale}-font{int(size)}")
                        frame = pixels(path)
                        current_bounds = largest_region(frame, width, panel)
                        current_save = largest_region(frame, width, accent)
                        if baseline is None:
                            baseline, bounds, save = frame, current_bounds, current_save
                        else:
                            self.assertEqual(current_bounds, bounds, "terminal font moved the settings panel")
                            self.assertEqual(current_save, save, "terminal font resized or moved Save")
                            left, top, right, bottom = bounds
                            changed = sum(frame[index:index + 3] != baseline[index:index + 3] for y in range(top, bottom + 1) for x in range(left, right + 1) for index in [(y * width + x) * 3])
                            self.assertLess(changed / ((right - left + 1) * (bottom - top + 1)), .02, "terminal font changed UI text rasterization")
                    original = terminal.config.read_bytes()
                    terminal.tabs(12)
                    terminal.x11.chord("Return")
                    productivity.smoke.wait_for(lambda: terminal.runtime_config()["font"]["size"] == 36.5, "font preview works with independent UI cells")
                    self.assertEqual(terminal.config.read_bytes(), original)
                    terminal.x11.chord("Escape")
                    productivity.smoke.wait_for(lambda: terminal.runtime_config()["font"]["size"] == 36., "Cancel restores terminal font")
                    terminal.preferences()
                    terminal.tabs(12)
                    terminal.x11.chord("Return")
                    productivity.smoke.wait_for(lambda: terminal.runtime_config()["font"]["size"] == 36.5, "second preview")
                    left, top, right, bottom = save
                    terminal.x11.click((left + right) / 2, (top + bottom) / 2)
                    productivity.smoke.wait_for(lambda: tomllib.loads(terminal.config.read_text()).get("font", {}).get("size") == 36.5, "mouse Save uses UI coordinates")
                    terminal.capture(f"terminal-{width}x{height}-dpi{scale}-after-save")
                finally:
                    terminal.close()
                    terminal.xvfb.stdout.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--screenshot", type=Path)
    args = parser.parse_args()
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], text=True))
    BINARY = str(args.binary.resolve() if args.binary else Path(metadata["target_directory"]) / "debug/termust")
    SCREENSHOT = args.screenshot
    unittest.main(argv=[__file__], verbosity=2)
