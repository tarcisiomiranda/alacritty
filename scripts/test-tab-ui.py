import argparse
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


spec = importlib.util.spec_from_file_location("tab_smoke", Path(__file__).with_name("test-tabs.py"))
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)


def run(binary, screenshot):
    for executable in ["Xvfb", "import", "convert", binary]:
        if not shutil.which(executable):
            raise RuntimeError(f"Missing executable: {executable}")
    with tempfile.TemporaryDirectory(prefix="alacritty-tab-ui-") as temporary:
        work = Path(temporary)
        home = work / "home"
        home.mkdir()
        config = work / "config.toml"
        config.write_text('''[general]
confirm_tab_close = false
live_config_reload = false
ipc_socket = false
[window]
title = "tab-ui"
dynamic_title = true
decorations = "None"
dimensions = { columns = 100, lines = 32 }
padding = { x = 0, y = 0 }
dynamic_padding = false
[terminal.shell]
program = "/bin/bash"
args = ["--noprofile", "--norc"]
''')
        mouse_probe = work / "mouse-probe.py"
        mouse_probe.write_text('''import os
from pathlib import Path
import select
import sys
import termios
import time
import tty

directory = Path(sys.argv[1])
fd = sys.stdin.fileno()
old = termios.tcgetattr(fd)
tty.setraw(fd)
os.write(sys.stdout.fileno(), b"\\x1b[2J\\x1b[H\\x1b[?1000h\\x1b[?1006h")
(directory / "mouse-ready").touch()
received = b""
try:
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if select.select([fd], [], [], 0.1)[0]:
            received += os.read(fd, 4096)
            (directory / "mouse-events").write_bytes(received)
            if b"\\x03" in received:
                break
finally:
    os.write(sys.stdout.fileno(), b"\\x1b[?1000l\\x1b[?1006l")
    termios.tcsetattr(fd, termios.TCSANOW, old)
    (directory / "mouse-finished").touch()
''')
        xvfb = subprocess.Popen(["Xvfb", "-displayfd", "1", "-screen", "0", "1200x900x24", "-ac"],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        app = None
        x11 = None
        log = (work / "app.log").open("w")
        try:
            ready, _, _ = select.select([xvfb.stdout], [], [], 5)
            if not ready:
                raise RuntimeError("Xvfb did not start")
            display = ":" + xvfb.stdout.readline().strip()
            env = dict(os.environ, DISPLAY=display, WINIT_UNIX_BACKEND="x11", LIBGL_ALWAYS_SOFTWARE="1",
                XDG_STATE_HOME=str(work / "state"), HOME=str(home))
            env.pop("WAYLAND_DISPLAY", None)
            env.pop("BASH_ENV", None)
            app = subprocess.Popen([binary, "--config-file", str(config)], env=env, stdout=log, stderr=log)
            x11 = smoke.X11(display)
            window = smoke.wait_for(lambda: next((w for w in x11.windows() if x11.title(w) == "tab-ui"), None),
                "terminal window opened")
            time.sleep(0.5)
            x11.lib.XSetInputFocus(x11.display, window, 2, 0)
            x11.lib.XFlush(x11.display)
            state_file = work / "state" / "alacritty" / "tabs.json"
            size_file = work / "size"

            def command(text):
                x11.type(text + "\n")

            def dimensions():
                rows, columns = map(int, size_file.read_text().split())
                width, height = x11.geometry(window)
                return columns, rows, width / columns, height / (rows + 1)

            def click(column, line, button=1):
                _, _, cell_width, cell_height = dimensions()
                x11.click((column + 0.5) * cell_width, (line + 1.5) * cell_height, button)

            def terminal_menu():
                columns, rows, _, _ = dimensions()
                click(columns - 1, rows - 1, 3)
                return columns - min(50, columns), rows - min(23, rows)

            def menu_item(origin, row):
                _, rows, _, _ = dimensions()
                height = min(23, rows)
                if height == 23:
                    click(origin[0] + 4, origin[1] + row)
                    return
                header = int(height >= 8)
                capacity = height - header - 2
                logical_row = row - 3
                offset = 0
                if logical_row >= capacity:
                    offset = 19 - capacity
                    for _ in range(offset):
                        click(origin[0] + 4, origin[1] + 3, 5)
                click(origin[0] + 4, origin[1] + header + 1 + logical_row - offset)

            def dialog_origin():
                columns, rows, _, _ = dimensions()
                width = min(76, columns - 4 if columns >= 12 else columns)
                height = min(24, rows)
                return (columns - width) // 2, (rows - height) // 2

            def dialog_apply():
                columns, rows, _, _ = dimensions()
                left, top = dialog_origin()
                width = min(76, columns - 4 if columns >= 12 else columns)
                wide = width - 4 >= 54
                click(left + 6, top + min(24, rows) - (5 if wide else 7))

            def capture(label=None):
                path = work / ((label or "dialog") + ".png")
                subprocess.run(["import", "-display", display, "-window", str(window), str(path)],
                    check=True, timeout=5)
                if screenshot:
                    destination = Path(screenshot)
                    if label:
                        destination = destination.with_name(destination.stem + "." + label + destination.suffix)
                    shutil.copyfile(path, destination)
                return path

            def menu_body_pixels(path, origin):
                _, rows, cell_width, cell_height = dimensions()
                height = min(23, rows)
                header = int(height >= 8)
                capacity = height - header - 2
                geometry = f"{int(40 * cell_width)}x{int(capacity * cell_height)}+{int((origin[0] + 1) * cell_width)}+{int((origin[1] + header + 2) * cell_height)}"
                return subprocess.run(["convert", str(path), "-crop", geometry, "+repage", "-depth", "8",
                    "rgb:-"], capture_output=True, check=True, timeout=5).stdout

            def bar_close_pixels(path, column):
                _, _, cell_width, cell_height = dimensions()
                geometry = f"{int(2 * cell_width)}x{int(cell_height)}+{int(column * cell_width)}+0"
                return subprocess.run(["convert", str(path), "-crop", geometry, "+repage", "-depth", "8",
                    "rgb:-"], capture_output=True, check=True, timeout=5).stdout

            def saved_tabs(count):
                return smoke.wait_for(lambda: json.loads(state_file.read_text())
                    if state_file.exists() and len(json.loads(state_file.read_text())["tabs"]) == count else None,
                    f"saved session contains {count} tabs")

            command(f"cd {shlex.quote(str(work))}; printf '\\033]0;FIRST\\007'; stty size > {size_file}")
            smoke.wait_for(lambda: size_file.exists() and x11.title(window) == "FIRST", "first tab initialized")
            command("pwd")
            command("printf 'Build concluido\\n'")
            command("ls")
            time.sleep(0.3)
            origin = terminal_menu()
            menu_item(origin, 12)
            x11.chord("q")
            menu_item(origin, 4)
            x11.type("Backend")
            capture()
            left, top = dialog_origin()
            click(left + 5, top + 16)
            dialog_apply()
            smoke.wait_for(lambda: x11.title(window) == "Backend", "right-click rename applies through mouse button")
            origin = terminal_menu()
            capture("menu")
            menu_item(origin, 15)
            snapshot = saved_tabs(1)
            assert snapshot["tabs"][0]["title"] == "Backend"
            assert snapshot["tabs"][0]["replay_last_command"] is True
            x11.chord("Escape")
            print("ok: terminal context menu, mouse rename and save session", flush=True)

            menu_item(terminal_menu(), 3)
            smoke.wait_for(lambda: x11.title(window) == "tab-ui", "menu creates a tab")
            command(f"printf '\\033]0;Worker\\007'; echo $$ > {work}/worker-pid")
            smoke.wait_for(lambda: (work / "worker-pid").exists() and x11.title(window) == "Worker", "second tab initialized")
            _, _, cell_width, cell_height = dimensions()
            x11.xtest.XTestFakeMotionEvent(x11.display, -1, int(5 * cell_width), int(8 * cell_height), 0)
            x11.lib.XFlush(x11.display)
            time.sleep(0.1)
            idle_close = bar_close_pixels(capture("tabs-idle"), 27)
            x11.xtest.XTestFakeMotionEvent(x11.display, -1, int(28 * cell_width), int(cell_height / 2), 0)
            x11.lib.XFlush(x11.display)
            time.sleep(0.1)
            assert bar_close_pixels(capture("tabs-hover-close"), 27) != idle_close, \
                "Hover did not expose the inactive tab close target"
            assert x11.title(window) == "Worker"
            print("ok: compact tab bar hover exposes closing without switching sessions", flush=True)
            x11.click(2.5 * cell_width, cell_height / 2, 3)
            assert x11.title(window) == "Worker"
            menu_item((2, 0), 4)
            x11.chord("Control_L", "a")
            x11.type("API")
            dialog_apply()
            assert x11.title(window) == "Worker"
            x11.chord("Control_L", "Shift_L", "s")
            snapshot = saved_tabs(2)
            assert snapshot["active"] == 1
            assert [tab["title"] for tab in snapshot["tabs"]] == ["API", None]
            x11.chord("Escape")
            x11.chord("Alt_L", "1")
            smoke.wait_for(lambda: x11.title(window) == "API", "inactive tab alone receives new name")
            x11.chord("Alt_L", "2")
            smoke.wait_for(lambda: x11.title(window) == "Worker", "active tab retained its shell title")
            x11.click(2.5 * cell_width, cell_height / 2, 3)
            menu_item((2, 0), 21)
            assert x11.title(window) == "Worker"
            x11.chord("Control_L", "Shift_L", "s")
            saved_tabs(1)
            x11.chord("Escape")
            print("ok: inactive tab actions preserve active session and target identity", flush=True)

            columns, _, _, _ = dimensions()
            toolbar_column = min(30, columns - 6) + 4
            x11.click((toolbar_column + 0.5) * cell_width, cell_height / 2)
            menu_item((min(toolbar_column, columns - 50), 0), 4)
            x11.chord("Control_L", "a")
            x11.type("Console")
            dialog_apply()
            smoke.wait_for(lambda: x11.title(window) == "Console", "toolbar opens same context menu")

            dismissal = work / "dismissal"
            x11.type(f"printf clean > {dismissal}")
            terminal_menu()
            x11.chord("q")
            x11.chord("Escape")
            terminal_menu()
            click(0, 0)
            x11.chord("Return")
            smoke.wait_for(dismissal.exists, "dismissed menus preserve unfinished shell command")
            assert dismissal.read_text() == "clean"
            print("ok: Esc and outside click dismiss without passing menu input to PTY", flush=True)

            clipboard_marker = work / "clipboard"
            clipboard_text = f"echo clipboard-ui > {clipboard_marker}"
            command(f"printf '\\033[2J\\033[5;1H{clipboard_text}\\033[10;1H'")
            time.sleep(0.2)
            x11.xtest.XTestFakeMotionEvent(x11.display, -1, int(0.25 * cell_width), int(5.5 * cell_height), 0)
            x11.xtest.XTestFakeButtonEvent(x11.display, 1, True, 0)
            x11.xtest.XTestFakeMotionEvent(x11.display, -1, int((len(clipboard_text) + 0.25) * cell_width),
                int(5.5 * cell_height), 0)
            x11.xtest.XTestFakeButtonEvent(x11.display, 1, False, 0)
            x11.lib.XFlush(x11.display)
            time.sleep(0.1)
            menu_item(terminal_menu(), 12)
            menu_item(terminal_menu(), 13)
            x11.chord("Return")
            smoke.wait_for(clipboard_marker.exists, "menu Copy and Paste use selected terminal text")
            assert clipboard_marker.read_text() == "clipboard-ui\n"
            print("ok: context menu copies selection and pastes through terminal input", flush=True)

            command(f"python3 {mouse_probe} {work}")
            smoke.wait_for(lambda: (work / "mouse-ready").exists(), "application enabled terminal mouse reporting")
            terminal_menu()
            smoke.wait_for(lambda: (work / "mouse-events").exists()
                and b"\x1b[<2;" in (work / "mouse-events").read_bytes(), "right click reaches mouse-aware application")
            time.sleep(0.2)
            initial_events = (work / "mouse-events").read_bytes()
            shift = x11.keycode("Shift_L")
            x11.press(shift, True)
            origin = terminal_menu()
            x11.press(shift, False)
            x11.lib.XFlush(x11.display)
            x11.chord("q")
            x11.chord("Escape")
            time.sleep(0.2)
            assert (work / "mouse-events").read_bytes() == initial_events
            x11.press(shift, True)
            origin = terminal_menu()
            x11.press(shift, False)
            x11.lib.XFlush(x11.display)
            menu_item(origin, 4)
            x11.chord("Control_L", "a")
            x11.type("Mouse application")
            dialog_apply()
            smoke.wait_for(lambda: x11.title(window) == "Mouse application", "Shift right click opens menu during mouse reporting")
            x11.chord("Control_L", "c")
            smoke.wait_for(lambda: (work / "mouse-finished").exists(), "mouse-aware fixture exited")
            print("ok: mouse-aware applications keep right click; Shift opens context menu", flush=True)

            x11.lib.XResizeWindow(x11.display, window, int(45 * cell_width), int(19 * cell_height))
            x11.lib.XFlush(x11.display)
            time.sleep(0.3)
            size_file.unlink()
            command(f"stty size > {size_file}")
            smoke.wait_for(size_file.exists, "compact terminal geometry reported")
            columns, rows, _, _ = dimensions()
            assert columns == 45 and rows == 18
            menu_item(terminal_menu(), 4)
            x11.chord("Control_L", "a")
            x11.type("Compact")
            capture("compact")
            left, top = dialog_origin()
            click(left + 5, top + min(24, rows) - 5)
            assert x11.title(window) == "Mouse application"
            x11.chord("Control_L", "Shift_L", "r")
            x11.chord("Control_L", "a")
            x11.type("Compact")
            dialog_apply()
            smoke.wait_for(lambda: x11.title(window) == "Compact", "compact dialog mouse buttons remain reachable")
            x11.chord("Control_L", "Shift_L", "r")
            for _ in range(5):
                x11.chord("Tab")
            x11.chord("Return")
            command(f"echo keyboard-cancel > {work}/compact-keyboard")
            smoke.wait_for(lambda: (work / "compact-keyboard").exists(), "compact keyboard navigation cancels dialog")
            print("ok: compact layout keeps mouse and keyboard controls reachable", flush=True)
            x11.chord("Control_L", "Shift_L", "s")
            saved_tabs(1)
            x11.chord("Escape")
            for name in ("mouse-ready", "mouse-events", "mouse-finished"):
                (work / name).unlink(missing_ok=True)
            command(f"python3 {mouse_probe} {work}")
            smoke.wait_for(lambda: (work / "mouse-ready").exists(), "compact raw mouse probe enabled")
            x11.press(shift, True)
            origin = terminal_menu()
            x11.press(shift, False)
            x11.lib.XFlush(x11.display)
            menu_item(origin, 12)
            x11.chord("q")
            top_pixels = menu_body_pixels(capture("compact-menu-top"), origin)
            click(origin[0] + 20, origin[1] + rows - 1)
            down_pixels = menu_body_pixels(capture("compact-menu-down-button"), origin)
            assert down_pixels != top_pixels, "Compact menu down button did not scroll"
            click(origin[0] + 20, origin[1] + 1)
            assert menu_body_pixels(capture("compact-menu-up-button"), origin) == top_pixels, \
                "Compact menu up button did not restore the top rows"
            for _ in range(4):
                click(origin[0] + 20, origin[1] + 5, 5)
            assert menu_body_pixels(capture("compact-menu-wheel"), origin) != top_pixels, \
                "Mouse wheel did not scroll the compact menu"
            x11.chord("Escape")
            time.sleep(0.15)
            received = (work / "mouse-events").read_bytes() if (work / "mouse-events").exists() else b""
            assert received == b"", ("Compact menu input leaked into application", received)
            x11.chord("Control_L", "c")
            smoke.wait_for(lambda: (work / "mouse-finished").exists(), "compact raw mouse probe exited")
            print("ok: disabled items stay open; compact wheel/arrows scroll without PTY input", flush=True)
            origin = terminal_menu()
            for _ in range(4):
                click(origin[0] + 20, origin[1] + 5, 5)
            click(origin[0] + 4, origin[1] + 11)
            time.sleep(0.3)
            x11.chord("Control_L", "Shift_L", "s")
            snapshot = saved_tabs(2)
            assert snapshot["active"] == 1
            assert [tab["title"] for tab in snapshot["tabs"]] == ["Compact", "Compact"]
            x11.chord("Escape")
            print("ok: menu restores saved tabs alongside existing sessions", flush=True)
            x11.chord("Control_L", "Shift_L", "w")
            x11.chord("Control_L", "Shift_L", "w")
            smoke.wait_for(lambda: app.poll() == 0, "window closed")
        except Exception:
            print(f"app exit: {app.poll() if app else None}; titles: {[x11.title(w) for w in x11.windows()] if x11 else []}", flush=True)
            log.flush()
            print((work / "app.log").read_text(), flush=True)
            raise
        finally:
            if app and app.poll() is None:
                app.terminate()
                app.wait(timeout=5)
            if x11:
                x11.lib.XCloseDisplay(x11.display)
            log.close()
            xvfb.terminate()
            xvfb.wait(timeout=5)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", nargs="?")
    parser.add_argument("--screenshot")
    args = parser.parse_args()
    if args.binary:
        binary = Path(args.binary).resolve()
    else:
        metadata = json.loads(subprocess.run(["mise", "exec", "--", "cargo", "metadata", "--no-deps", "--format-version", "1"],
            check=True, capture_output=True, text=True).stdout)
        binary = Path(metadata["target_directory"]) / "debug" / "alacritty"
    run(str(binary), args.screenshot)
