import argparse
import importlib.util
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import time


def load(name, filename):
    source = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    result = importlib.util.module_from_spec(source)
    source.loader.exec_module(result)
    return result


productivity = load("codex_native_productivity", "test-productivity.py")
smoke = productivity.smoke

AUDIT = r'''#!/usr/bin/env python3
import fcntl,json,os,pty,re,sys,termios
if any(arg in {"--help","-h","--version","-V"} for arg in sys.argv[1:]):
    os.execv(os.environ["TERMUST_REAL_CODEX"],[os.environ["TERMUST_REAL_CODEX"]]+sys.argv[1:])
pattern=re.compile(rb"\x1b\]7501;([^\x1b\x07]{1,256})(?:\x07|\x1b\\)")
tail=b""
size=fcntl.ioctl(sys.stdin.fileno(),termios.TIOCGWINSZ,b"\0"*8)
resized=False
def read(fd):
    global tail,resized
    if not resized:
        fcntl.ioctl(fd,termios.TIOCSWINSZ,size)
        resized=True
    chunk=os.read(fd,4096)
    data=tail+chunk
    with open(os.environ["TERMUST_CODEX_NATIVE_RECORD"],"a") as output:
        for match in pattern.finditer(data):
            if match.end()<=len(tail):continue
            fields=dict(field.split("=",1) for field in match[1].decode("ascii").split(":"))
            output.write(json.dumps({key:fields[key] for key in ("app","state","kind") if key in fields})+"\n")
    tail=data[-512:]
    return chunk
result=pty.spawn([os.environ["TERMUST_REAL_CODEX"]]+sys.argv[1:],master_read=read)
with open(os.environ["TERMUST_CODEX_NATIVE_EXITED"],"w") as output:
    output.write(str(os.waitstatus_to_exitcode(result)))
raise SystemExit(os.waitstatus_to_exitcode(result))
'''


def states(path):
    if not path.exists():
        return []
    result = []
    for line in path.read_text().splitlines():
        try:
            result.append(json.loads(line))
        except json.JSONDecodeError:
            pass
    return result


def wait_state(path, state, start=0, kind=None):
    return smoke.wait_for(lambda: next((record for record in states(path)[start:]
        if record.get("state") == state and (kind is None or record.get("kind") == kind)), None),
        "real Codex native state " + state + ("/" + kind if kind else ""), timeout=45)


def overview(terminal):
    columns, _, cell_width, cell_height = terminal.dimensions()
    width = min(30, max(6, columns - 9))
    terminal.x11.click((width + 7) * cell_width, cell_height / 2)
    time.sleep(.2)


def prompt(terminal, text):
    terminal.x11.type(text)
    time.sleep(.3)
    terminal.x11.chord("Return")
    time.sleep(.3)
    terminal.x11.chord("Return")


def run(binary, screenshot, auth, permission):
    real_codex = shutil.which("codex")
    if real_codex is None or not auth.is_file():
        raise SystemExit("This opt-in test needs an installed, already logged-in Codex CLI with file-based auth.")
    with tempfile.TemporaryDirectory(prefix="termust-codex-native-ui-") as directory:
        terminal = productivity.Fixture(binary, Path(directory), screenshot)
        home = terminal.work / "codex-home"
        home.mkdir(mode=0o700)
        (home / "auth.json").symlink_to(auth)
        audit_bin = terminal.work / "audit-bin"
        audit_bin.mkdir(mode=0o700)
        audit = audit_bin / "codex"
        audit.write_text(AUDIT)
        audit.chmod(0o700)
        record = terminal.work / "native-events.jsonl"
        record.touch(mode=0o600)
        exited = terminal.work / "codex-exited"
        terminal.env.update(CODEX_HOME=str(home), TERMUST_REAL_CODEX=real_codex,
            TERMUST_CODEX_NATIVE_RECORD=str(record), TERMUST_CODEX_NATIVE_EXITED=str(exited),
            WINIT_X11_SCALE_FACTOR="1.0")
        terminal.env["PATH"] = str(audit_bin) + os.pathsep + terminal.env["PATH"]
        try:
            terminal.start()
            terminal.general(agent_notifications=False)
            terminal.rename("Codex Live")
            terminal.command("codex --no-daemon --no-alt-screen -s read-only")
            wait_state(record, "idle")
            terminal.capture("codex-native-ready")
            start = len(states(record))
            prompt(terminal, "Reply exactly TERMUST_CODEX_UI_OK. Do not use any tools or read files.")
            wait_state(record, "working", start)
            overview(terminal)
            terminal.capture("codex-native-working")
            wait_state(record, "done", start)
            time.sleep(.3)
            terminal.capture("codex-native-done")
            terminal.x11.chord("Escape")
            if permission:
                destination = terminal.work / "permission-check.txt"
                start = len(states(record))
                prompt(terminal, "Use a shell command to write exactly TERMUST_PERMISSION_TEST to "
                    + str(destination) + ". If permission is required, request it. Do not edit any other files.")
                wait_state(record, "blocked", start, "permission")
                overview(terminal)
                terminal.capture("codex-native-permission")
                terminal.x11.chord("Escape")
                terminal.x11.chord("Escape")
                wait_state(record, "idle", start)
                assert not destination.exists(), "Canceled command unexpectedly created its target"
            terminal.x11.chord("Control_L", "c")
            terminal.x11.chord("Control_L", "c")
            wait_state(record, "clear")
            smoke.wait_for(exited.exists, "real Codex exited and released its terminal")
            time.sleep(.3)
            terminal.report("codex-native-returned-to-shell")
            assert not (home / "hooks.json").exists()
            config = home / "config.toml"
            assert not config.exists() or "trusted_hash" not in config.read_text()
            result = {"codex_version": subprocess.check_output([real_codex, "--version"], text=True).strip(),
                "states": states(record), "hook_trust_used": False, "tool_approval_bypass_used": False}
            if screenshot:
                evidence = Path(screenshot).with_suffix(".native-evidence.json")
                evidence.parent.mkdir(parents=True, exist_ok=True)
                evidence.write_text(json.dumps(result, indent=2) + "\n")
            print(json.dumps(result, indent=2), flush=True)
        except BaseException:
            if terminal.app and terminal.app.poll() is None:
                terminal.capture("codex-native-failure")
            print("native metadata before failure:", json.dumps(states(record)), flush=True)
            raise
        finally:
            terminal.close()
            terminal.xvfb.stdout.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--live", action="store_true", help="Run real logged-in Codex; sends a small model request")
    parser.add_argument("--permission", action="store_true", help="Also observe and cancel a real permission request")
    parser.add_argument("--binary")
    parser.add_argument("--screenshot", type=Path)
    parser.add_argument("--auth-file", type=Path,
        default=Path(os.environ.get("CODEX_HOME", str(Path.home() / ".codex"))) / "auth.json")
    args = parser.parse_args()
    if not args.live:
        raise SystemExit("Live Codex smoke test is opt-in. Pass --live to use the existing login.")
    if args.binary:
        binary = Path(args.binary).resolve()
    else:
        metadata = json.loads(subprocess.run(["mise", "exec", "--", "cargo", "metadata", "--no-deps", "--format-version", "1"], check=True, capture_output=True, text=True).stdout)
        binary = Path(metadata["target_directory"]) / "debug" / "termust"
    run(str(binary), args.screenshot, args.auth_file.resolve(), args.permission)
