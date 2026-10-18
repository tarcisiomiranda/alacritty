import argparse
import ctypes
import ctypes.util
import json
import os
from pathlib import Path
import select
import shutil
import subprocess
import tempfile
import time


def wait_for(check, description, timeout=8):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = check()
        if result:
            print(f"ok: {description}", flush=True)
            return result
        time.sleep(0.05)
    raise AssertionError(description)


class X11:
    def __init__(self, display_name):
        self.lib = ctypes.CDLL(ctypes.util.find_library("X11"))
        self.xtest = ctypes.CDLL(ctypes.util.find_library("Xtst"))
        self.bind("XOpenDisplay", [ctypes.c_char_p], ctypes.c_void_p)
        self.bind("XDefaultRootWindow", [ctypes.c_void_p], ctypes.c_ulong)
        self.bind("XQueryTree", [ctypes.c_void_p, ctypes.c_ulong,
            ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_ulong),
            ctypes.POINTER(ctypes.POINTER(ctypes.c_ulong)), ctypes.POINTER(ctypes.c_uint)])
        self.bind("XFetchName", [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_void_p)])
        self.bind("XFree", [ctypes.c_void_p])
        self.bind("XSetInputFocus", [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong])
        self.bind("XStringToKeysym", [ctypes.c_char_p], ctypes.c_ulong)
        self.bind("XKeysymToKeycode", [ctypes.c_void_p, ctypes.c_ulong], ctypes.c_uint)
        self.bind("XKeycodeToKeysym", [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int], ctypes.c_ulong)
        self.bind("XFlush", [ctypes.c_void_p])
        self.bind("XResizeWindow", [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_uint, ctypes.c_uint])
        self.bind("XGetGeometry", [ctypes.c_void_p, ctypes.c_ulong,
            ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_uint), ctypes.POINTER(ctypes.c_uint),
            ctypes.POINTER(ctypes.c_uint), ctypes.POINTER(ctypes.c_uint)])
        self.bind("XCloseDisplay", [ctypes.c_void_p])
        self.xtest.XTestFakeKeyEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
        self.xtest.XTestFakeButtonEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
        self.xtest.XTestFakeMotionEvent.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]
        self.display = self.lib.XOpenDisplay(display_name.encode())
        if not self.display:
            raise RuntimeError("Cannot open test X11 display")
        self.root = self.lib.XDefaultRootWindow(self.display)

    def bind(self, name, args, result=ctypes.c_int):
        function = getattr(self.lib, name)
        function.argtypes = args
        function.restype = result

    def title(self, window):
        name = ctypes.c_void_p()
        if not self.lib.XFetchName(self.display, window, ctypes.byref(name)) or not name.value:
            return ""
        title = ctypes.string_at(name).decode(errors="replace")
        self.lib.XFree(name)
        return title

    def windows(self):
        root, parent = ctypes.c_ulong(), ctypes.c_ulong()
        children = ctypes.POINTER(ctypes.c_ulong)()
        count = ctypes.c_uint()
        self.lib.XQueryTree(self.display, self.root, ctypes.byref(root), ctypes.byref(parent),
            ctypes.byref(children), ctypes.byref(count))
        result = list(children[:count.value])
        if children:
            self.lib.XFree(children)
        return result

    def keycode(self, name):
        keysym = self.lib.XStringToKeysym(name.encode())
        keycode = self.lib.XKeysymToKeycode(self.display, keysym)
        if not keycode:
            raise RuntimeError(f"Unknown test key {name}")
        return keycode

    def press(self, keycode, down):
        self.xtest.XTestFakeKeyEvent(self.display, keycode, down, 0)

    def chord(self, *keys, pause=0.08):
        codes = [self.keycode(key) for key in keys]
        for code in codes:
            self.press(code, True)
        for code in reversed(codes):
            self.press(code, False)
        self.lib.XFlush(self.display)
        time.sleep(pause)

    def type(self, text):
        shift = self.keycode("Shift_L")
        for ch in text:
            keysym = 0xff0d if ch == "\n" else ord(ch)
            keycode = self.lib.XKeysymToKeycode(self.display, keysym)
            if not keycode:
                raise RuntimeError(f"Unsupported test character {ch!r}")
            shifted = self.lib.XKeycodeToKeysym(self.display, keycode, 0) != keysym
            if shifted:
                self.press(shift, True)
            self.press(keycode, True)
            self.press(keycode, False)
            if shifted:
                self.press(shift, False)
        self.lib.XFlush(self.display)

    def click(self, x, y, button=1):
        self.xtest.XTestFakeMotionEvent(self.display, -1, int(x), int(y), 0)
        self.xtest.XTestFakeButtonEvent(self.display, button, True, 0)
        self.xtest.XTestFakeButtonEvent(self.display, button, False, 0)
        self.lib.XFlush(self.display)
        time.sleep(0.1)

    def geometry(self, window):
        root = ctypes.c_ulong()
        x, y = ctypes.c_int(), ctypes.c_int()
        width, height, border, depth = (ctypes.c_uint() for _ in range(4))
        self.lib.XGetGeometry(self.display, window, ctypes.byref(root), ctypes.byref(x),
            ctypes.byref(y), ctypes.byref(width), ctypes.byref(height), ctypes.byref(border), ctypes.byref(depth))
        return width.value, height.value


def run(binary, screenshot):
    for executable in ["Xvfb", binary]:
        if not shutil.which(executable):
            raise RuntimeError(f"Missing executable: {executable}")
    with tempfile.TemporaryDirectory(prefix="alacritty-tabs-") as temporary:
        work = Path(temporary)
        config = work / "config.toml"
        config.write_text('''[general]
confirm_tab_close = false
live_config_reload = false
ipc_socket = false
[window]
title = "tabs-smoke"
dynamic_title = true
decorations = "None"
dimensions = { columns = 80, lines = 24 }
padding = { x = 0, y = 0 }
dynamic_padding = false
[terminal.shell]
program = "/bin/bash"
args = ["--noprofile", "--norc"]
''')
        probe = work / "probe.py"
        probe.write_text('''import os, select, sys, termios, time, tty
fd = sys.stdin.fileno()
old = termios.tcgetattr(fd)
tty.setraw(fd)
try:
    time.sleep(0.8)
    os.write(sys.stdout.fileno(), b"\\x1b[6n\\x1b]10;?\\x07\\x1b[18t")
    reply = b""
    end = time.monotonic() + 4
    while time.monotonic() < end:
        if select.select([fd], [], [], 0.1)[0]:
            reply += os.read(fd, 4096)
            if b"R" in reply and b"rgb:" in reply and reply.endswith(b"t"):
                break
    open(sys.argv[1], "wb").write(reply)
finally:
    termios.tcsetattr(fd, termios.TCSANOW, old)
''')
        xvfb = subprocess.Popen(["Xvfb", "-displayfd", "1", "-screen", "0", "1200x900x24", "-ac"],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        app = None
        x11 = None
        try:
            ready, _, _ = select.select([xvfb.stdout], [], [], 5)
            if not ready:
                raise RuntimeError("Xvfb did not start")
            display = ":" + xvfb.stdout.readline().strip()
            env = dict(os.environ, DISPLAY=display, WINIT_UNIX_BACKEND="x11", LIBGL_ALWAYS_SOFTWARE="1",
                XDG_STATE_HOME=str(work / "state"))
            env.pop("WAYLAND_DISPLAY", None)
            env.pop("BASH_ENV", None)
            with (work / "app.log").open("w") as log:
                app = subprocess.Popen([binary, "--config-file", str(config)], env=env, stdout=log, stderr=log)
            x11 = X11(display)
            window = wait_for(lambda: next((w for w in x11.windows() if x11.title(w) == "tabs-smoke"), None),
                "terminal window opened")
            time.sleep(0.5)
            x11.lib.XSetInputFocus(x11.display, window, 2, 0)
            x11.lib.XFlush(x11.display)
            first = work / "first"
            first.mkdir()
            x11.type(f"cd {first}; export TAB_TEST=first; printf '\\033]0;FIRST\\007'; stty size > {work}/size1; echo $$ > {work}/pid1\n")
            wait_for(lambda: (work / "pid1").exists() and x11.title(window) == "FIRST", "first session initialized")
            x11.chord("Control_L", "Shift_L", "t")
            wait_for(lambda: x11.title(window) == "tabs-smoke", "keyboard creates a second tab")
            x11.type(f"pwd > {work}/cwd2; echo ${{TAB_TEST-unset}} > {work}/env2; echo $$ > {work}/pid2; printf '\\033]0;SECOND\\007'; stty size > {work}/size2\n")
            wait_for(lambda: (work / "size2").exists() and x11.title(window) == "SECOND", "second session initialized")
            assert (work / "cwd2").read_text().strip() == str(first)
            assert (work / "env2").read_text().strip() == "unset"
            assert (work / "pid1").read_text() != (work / "pid2").read_text()
            assert (work / "size1").read_text() == (work / "size2").read_text()
            print("ok: independent shells, working directory inheritance and equal terminal sizes", flush=True)
            x11.chord("Control_L", "Page_Up", pause=0)
            x11.type(f"echo ${{TAB_TEST-unset}} > {work}/rapid1\n")
            wait_for(lambda: (work / "rapid1").exists(), "input immediately after switching tabs")
            assert (work / "rapid1").read_text().strip() == "first"
            wait_for(lambda: x11.title(window) == "FIRST", "previous-tab shortcut")
            x11.type(f"python3 {probe} {work}/reply\n")
            x11.chord("Control_L", "Page_Down")
            wait_for(lambda: x11.title(window) == "SECOND", "next-tab shortcut")
            x11.type(f"echo second > {work}/input2\n")
            wait_for(lambda: (work / "reply").exists(), "background PTY receives terminal query replies")
            reply = (work / "reply").read_bytes()
            assert b"R" in reply and b"rgb:" in reply and reply.endswith(b"t"), repr(reply)
            assert (work / "input2").read_text().strip() == "second"
            x11.chord("Alt_L", "1")
            wait_for(lambda: x11.title(window) == "FIRST", "numeric tab selection")
            x11.type(f"trap 'stty size > {work}/resize1' WINCH\n")
            x11.chord("Alt_L", "9")
            wait_for(lambda: x11.title(window) == "SECOND", "last-tab shortcut")
            x11.type(f"trap 'stty size > {work}/resize2' WINCH\n")
            time.sleep(0.15)
            width, height = x11.geometry(window)
            rows, columns = map(int, (work / "size1").read_text().split())
            cell_width = width / columns
            cell_height = height / (rows + 1)
            x11.lib.XResizeWindow(x11.display, window, width + int(10 * cell_width), height + int(4 * cell_height))
            x11.lib.XFlush(x11.display)
            wait_for(lambda: (work / "resize1").exists() and (work / "resize2").exists(), "all PTYs resized")
            assert (work / "resize1").read_text() == (work / "resize2").read_text()
            new_rows, new_columns = map(int, (work / "resize1").read_text().split())
            assert (new_rows, new_columns) == (rows + 4, columns + 10)
            x11.type(f"printf '\\033[{new_rows};1HBOTTOM ROW VISIBLE'\n")
            time.sleep(0.15)
            if screenshot:
                subprocess.run(["import", "-display", display, "-window", str(window), screenshot], check=True)
            def drag_tab(source, target):
                test = x11.xtest
                test.XTestFakeMotionEvent(x11.display, -1, int(source * cell_width), int(cell_height / 2), 0)
                test.XTestFakeButtonEvent(x11.display, 1, True, 0)
                x11.lib.XFlush(x11.display)
                time.sleep(0.1)
                test.XTestFakeMotionEvent(x11.display, -1, int(target * cell_width), int(cell_height / 2), 0)
                x11.lib.XFlush(x11.display)
                time.sleep(0.1)
                test.XTestFakeButtonEvent(x11.display, 1, False, 0)
                x11.lib.XFlush(x11.display)
                time.sleep(0.1)

            x11.chord("Alt_L", "1")
            drag_tab(3, 45)
            assert x11.title(window) == "FIRST"
            x11.chord("Alt_L", "1")
            wait_for(lambda: x11.title(window) == "SECOND", "drag reorders tabs and preserves active session")
            x11.chord("Alt_L", "2")
            wait_for(lambda: x11.title(window) == "FIRST", "numeric selection follows the reordered tab")
            drag_tab(45, 3)
            assert x11.title(window) == "FIRST"
            x11.chord("Alt_L", "2")
            wait_for(lambda: x11.title(window) == "SECOND", "drag restores the original tab order")
            x11.click(cell_width, cell_height / 2)
            wait_for(lambda: x11.title(window) == "FIRST", "mouse selects a tab")
            x11.type("sleep 0.8; exit\n")
            x11.chord("Alt_L", "9")
            time.sleep(1.1)
            assert app.poll() is None and x11.title(window) == "SECOND"
            x11.chord("Control_L", "Page_Up")
            assert x11.title(window) == "SECOND"
            print("ok: background shell exit closes only its tab", flush=True)
            x11.click(31 * cell_width, cell_height / 2)
            wait_for(lambda: x11.title(window) == "tabs-smoke", "plus button creates a tab")
            x11.chord("Control_L", "Shift_L", "w")
            wait_for(lambda: x11.title(window) == "SECOND", "keyboard closes active tab")
            x11.click(29.5 * cell_width, cell_height / 2)
            assert app.poll() is None and x11.title(window) == "SECOND", \
                "The tab's outer margin must not close its session"
            x11.click(28 * cell_width, cell_height / 2)
            wait_for(lambda: app.poll() == 0, "close button on final tab closes window")
            with (work / "app.log").open("a") as log:
                app = subprocess.Popen([binary, "--config-file", str(config), "--title", "HELD", "--hold", "-e", "/bin/true"], env=env, stdout=log, stderr=log)
            window = wait_for(lambda: next((w for w in x11.windows() if x11.title(w) == "HELD"), None),
                "hold preserves exited session")
            time.sleep(0.5)
            x11.lib.XSetInputFocus(x11.display, window, 2, 0)
            x11.lib.XFlush(x11.display)
            x11.chord("Control_L", "Shift_L", "t")
            wait_for(lambda: x11.title(window) == "tabs-smoke", "live tab opens alongside held session")
            x11.type("exit\n")
            wait_for(lambda: x11.title(window) == "HELD", "new tab exits without inheriting hold")
            assert app.poll() is None
            x11.chord("Control_L", "Shift_L", "w")
            wait_for(lambda: app.poll() == 0, "held tab can be closed explicitly")
        except Exception:
            print(f"app exit: {app.poll() if app else None}; titles: {[x11.title(w) for w in x11.windows()] if x11 else []}", flush=True)
            print((work / "app.log").read_text(), flush=True)
            raise
        finally:
            if app and app.poll() is None:
                app.terminate()
                app.wait(timeout=5)
            if x11:
                x11.lib.XCloseDisplay(x11.display)
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
