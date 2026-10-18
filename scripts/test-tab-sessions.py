import argparse
import importlib.util
import json
import os
from pathlib import Path
import select
import shlex
import subprocess
import tempfile
import time


spec = importlib.util.spec_from_file_location("tab_smoke", Path(__file__).with_name("test-tabs.py"))
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)


def run(binary, screenshot):
    with tempfile.TemporaryDirectory(prefix="alacritty-session-") as temporary:
        work = Path(temporary)
        first = work / "project's one"
        second = work / "project two"
        first.mkdir()
        second.mkdir()
        home = work / "home"
        home.mkdir()
        config = work / "config.toml"
        config.write_text('''[general]
confirm_tab_close = false
live_config_reload = false
ipc_socket = false
[window]
title = "tabs-session"
dynamic_title = true
decorations = "None"
dimensions = { columns = 90, lines = 26 }
padding = { x = 0, y = 0 }
dynamic_padding = false
[terminal.shell]
program = "/bin/bash"
args = ["--noprofile", "--norc"]
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
            x11 = smoke.X11(display)
            state_file = work / "state" / "alacritty" / "tabs.json"

            def start(title, *options):
                nonlocal app
                app = subprocess.Popen([binary, "--config-file", str(config), *options], env=env, stdout=log, stderr=log)
                window = smoke.wait_for(lambda: next((w for w in x11.windows() if x11.title(w) == title), None),
                    f"window opened with {title}")
                time.sleep(0.4)
                x11.lib.XSetInputFocus(x11.display, window, 2, 0)
                x11.lib.XFlush(x11.display)
                return window

            def command(text):
                x11.type(text + "\n")

            def rename(window, name):
                x11.chord("Control_L", "Shift_L", "r")
                x11.type(name)
                x11.chord("Return")
                smoke.wait_for(lambda: x11.title(window) == name, f"renamed tab to {name}")

            def close_all():
                for _ in range(8):
                    if app.poll() is not None:
                        break
                    x11.chord("Escape")
                    x11.chord("Control_L", "Shift_L", "w")
                    time.sleep(0.1)
                smoke.wait_for(lambda: app.poll() == 0, "window closed without replacing saved session")

            window = start("tabs-session")
            command(f"cd {shlex.quote(str(first))}; printf '\\033]0;FIRST\\007'; echo $$ > {work}/pid1")
            smoke.wait_for(lambda: (work / "pid1").exists() and x11.title(window) == "FIRST", "first working directory")
            rename(window, "Backend")
            command("printf '\\033]0;changed-by-shell\\007'")
            time.sleep(0.2)
            assert x11.title(window) == "Backend"
            first_command = "printf 'first\\n' >> counts.txt"
            command(first_command)
            smoke.wait_for(lambda: (first / "counts.txt").exists(), "captured command executed in first tab")
            x11.chord("Control_L", "Shift_L", "r")
            x11.chord("Tab")
            x11.chord("space")
            x11.chord("Control_L", "Shift_L", "s")
            smoke.wait_for(state_file.exists, "session saved through settings dialog")
            assert json.loads(state_file.read_text())["tabs"][0]["replay_last_command"] is True
            x11.chord("Escape")
            x11.chord("Control_L", "Shift_L", "t")
            smoke.wait_for(lambda: x11.title(window) == "tabs-session", "second tab opened")
            command(f"cd {shlex.quote(str(second))}; printf '\\033]0;SECOND\\007'; echo $$ > {work}/pid2")
            smoke.wait_for(lambda: (work / "pid2").exists() and x11.title(window) == "SECOND", "second working directory")
            rename(window, "Worker")
            second_command = "printf 'second\\n' >> counts.txt"
            command(second_command)
            smoke.wait_for(lambda: (second / "counts.txt").exists(), "captured command executed in second tab")
            x11.chord("Control_L", "Shift_L", "s")
            snapshot = smoke.wait_for(lambda: json.loads(state_file.read_text())
                if state_file.exists() and len(json.loads(state_file.read_text())["tabs"]) == 2 else None,
                "both tabs saved")
            assert [tab["title"] for tab in snapshot["tabs"]] == ["Backend", "Worker"]
            assert [tab["working_directory"] for tab in snapshot["tabs"]] == [str(first), str(second)]
            assert snapshot["active"] == 1
            assert snapshot["tabs"][0]["commands"][-1]["command"] == first_command
            assert snapshot["tabs"][1]["commands"][-1]["command"] == second_command
            assert snapshot["tabs"][1]["replay_last_command"] is False
            assert all(tab["shell"]["program"] == "/bin/bash" for tab in snapshot["tabs"])
            assert state_file.stat().st_mode & 0o777 == 0o600
            if screenshot:
                subprocess.run(["import", "-display", display, "-window", str(window), screenshot], check=True)
            close_all()
            assert json.loads(state_file.read_text()) == snapshot

            window = start("Worker")
            smoke.wait_for(lambda: (first / "counts.txt").read_text() == "first\nfirst\n",
                "only opted-in command replayed after shell became ready")
            time.sleep(0.3)
            assert (second / "counts.txt").read_text() == "second\n"
            x11.chord("Control_L", "Shift_L", "s")
            time.sleep(0.2)
            replayed = json.loads(state_file.read_text())
            assert replayed["tabs"][0]["commands"][-1]["command"] == first_command
            assert replayed["tabs"][0]["replay_last_command"] is True
            assert replayed["tabs"][1]["replay_last_command"] is False
            x11.chord("Escape")
            command(f"pwd > {work}/restored2")
            smoke.wait_for(lambda: (work / "restored2").exists(), "second directory restored")
            assert (work / "restored2").read_text().strip() == str(second)
            x11.chord("Alt_L", "1")
            smoke.wait_for(lambda: x11.title(window) == "Backend", "names and tab order restored")
            command(f"pwd > {work}/restored1")
            smoke.wait_for(lambda: (work / "restored1").exists(), "first directory restored")
            assert (work / "restored1").read_text().strip() == str(first)
            x11.chord("Control_L", "Shift_L", "o")
            time.sleep(0.3)
            assert x11.title(window) == "Worker"
            x11.chord("Control_L", "Shift_L", "s")
            time.sleep(0.2)
            assert len(json.loads(state_file.read_text())["tabs"]) == 4
            x11.chord("Escape")
            print("ok: reopening saved tabs retains existing sessions", flush=True)
            close_all()
            state_file.write_text(json.dumps(snapshot))

            window = start("Explicit", "--title", "Explicit", "--hold", "-e", "/bin/true")
            time.sleep(0.3)
            assert (second / "counts.txt").read_text() == "second\n"
            close_all()
            print("ok: explicit startup options bypass automatic session restoration", flush=True)

            original_directory = json.loads(json.dumps(snapshot))
            original_directory["tabs"][0]["working_directory"] = str(work)
            original_directory["tabs"][0]["commands"][-1]["command"] = f"cd ..; pwd > {work}/replay-original-dir"
            state_file.write_text(json.dumps(original_directory))
            window = start("Worker")
            smoke.wait_for(lambda: (work / "replay-original-dir").exists(), "relative command replay")
            assert (work / "replay-original-dir").read_text().strip() == str(work)
            close_all()

            missing_directory = json.loads(json.dumps(snapshot))
            missing_directory["tabs"][0]["commands"][-1]["working_directory"] = str(work / "deleted")
            missing_directory["tabs"][0]["commands"][-1]["command"] = f"touch {work}/must-not-run"
            state_file.write_text(json.dumps(missing_directory))
            window = start("Worker")
            time.sleep(0.5)
            assert not (work / "must-not-run").exists()
            close_all()
            print("ok: absent command directory prevents replay", flush=True)

            cancel_replay = json.loads(json.dumps(snapshot))
            cancel_replay["tabs"][0]["shell"]["args"] = ["--noprofile"]
            cancel_replay["tabs"][0]["commands"][-1]["command"] = f"touch {work}/canceled-replay"
            gate = work / "shell-ready"
            (home / ".bashrc").write_text(f"while [[ ! -e {shlex.quote(str(gate))} ]]; do sleep 0.05; done\n")
            state_file.write_text(json.dumps(cancel_replay))
            window = start("Worker")
            x11.chord("Alt_L", "1")
            x11.chord("Control_L", "Shift_L", "r")
            x11.chord("Tab")
            x11.chord("space")
            x11.chord("Control_L", "Shift_L", "s")
            smoke.wait_for(lambda: json.loads(state_file.read_text())["tabs"][0]["replay_last_command"] is False,
                "disabling pending replay saves preference")
            x11.chord("Escape")
            gate.touch()
            command(f"pwd > {work}/cancel-restored-dir")
            smoke.wait_for(lambda: (work / "cancel-restored-dir").exists(), "delayed shell became usable")
            time.sleep(0.3)
            assert not (work / "canceled-replay").exists()
            close_all()
            print("ok: disabling replay before shell readiness cancels pending execution", flush=True)

            missing_shell = json.loads(json.dumps(snapshot))
            missing_shell["tabs"][0]["shell"]["program"] = str(work / "unavailable" / "bash")
            state_file.write_text(json.dumps(missing_shell))
            window = start("tabs-session")
            x11.chord("Escape")
            command(f"pwd > {work}/fallback-terminal")
            smoke.wait_for(lambda: (work / "fallback-terminal").exists(), "missing saved shell falls back to a usable terminal")
            close_all()
            assert json.loads(state_file.read_text()) == missing_shell

            state_file.write_text("invalid session JSON")
            window = start("tabs-session")
            close_all()
            assert state_file.read_text() == "invalid session JSON"
            print("ok: invalid snapshots preserve file and allow terminal startup", flush=True)
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
        metadata = json.loads(subprocess.run(["cargo", "metadata", "--no-deps", "--format-version", "1"],
            check=True, capture_output=True, text=True).stdout)
        binary = Path(metadata["target_directory"]) / "debug" / "alacritty"
    run(str(binary), args.screenshot)
