import contextlib
import fcntl
import json
import os
from pathlib import Path
import re
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time
import uuid


PROVIDERS = {"claude", "codex", "opencode", "agy"}
STATES = {"ready", "working", "input", "permission", "done", "error", "unknown"}
MAX_INPUT = 1024 * 1024
MAX_RECORD = 4096
CLAUDE_EVENTS = (
    "SessionStart", "UserPromptSubmit", "PreToolUse", "PermissionRequest",
    "PostToolUse", "PostToolUseFailure", "Stop", "StopFailure", "SessionEnd",
    "Notification", "Elicitation", "ElicitationResult",
)


def private_root():
    value = os.environ.get("TERMUST_AGENT_DIR", "")
    if not value or not os.path.isabs(value):
        raise ValueError("missing private directory")
    root = Path(value)
    info = root.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
        raise ValueError("unsafe private directory")
    return root


def run_id():
    value = os.environ.get("TERMUST_AGENT_RUN", "")
    if str(uuid.UUID(value)) != value:
        raise ValueError("invalid run")
    return value


def wrapper_pid():
    value = os.environ.get("TERMUST_AGENT_PID", "")
    if not value.isdecimal() or int(value) <= 1:
        raise ValueError("invalid process")
    return int(value)


def safe_open(path, flags, mode=0o600):
    flags |= getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0)
    fd = os.open(path, flags, mode)
    info = os.fstat(fd)
    if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid()
            or info.st_nlink != 1 or info.st_mode & 0o077):
        os.close(fd)
        raise ValueError("unsafe file")
    return fd


def read_record(path):
    try:
        fd = safe_open(path, os.O_RDONLY)
    except FileNotFoundError:
        return None
    with os.fdopen(fd, "rb") as handle:
        if os.fstat(handle.fileno()).st_size > MAX_RECORD:
            raise ValueError("oversized record")
        record = json.loads(handle.read(MAX_RECORD + 1))
    if not isinstance(record, dict):
        raise ValueError("invalid record")
    return record


def update(provider, state, limited=False, transform=None):
    if provider not in PROVIDERS or state not in STATES:
        raise ValueError("invalid state")
    root, identity, pid = private_root(), run_id(), wrapper_pid()
    path = root / (identity + ".json")
    lock = safe_open(root / (identity + ".lock"), os.O_RDWR | os.O_CREAT)
    with os.fdopen(lock, "r+b") as handle:
        fcntl.flock(handle, fcntl.LOCK_EX)
        previous = read_record(path)
        sequence = 0
        if previous is not None:
            if (previous.get("v") != 1 or previous.get("run") != identity
                    or previous.get("provider") != provider or previous.get("pid") != pid):
                raise ValueError("mismatched record")
            sequence = previous.get("sequence", 0)
            if type(sequence) is not int or not 0 <= sequence < (1 << 63) - 1:
                raise ValueError("invalid sequence")
        if transform is not None:
            state, limited = transform(previous)
        if state not in STATES:
            raise ValueError("invalid transformed state")
        record = {"v": 1, "provider": provider, "state": state, "run": identity,
                  "pid": pid, "sequence": sequence + 1,
                  "updated_ms": time.time_ns() // 1_000_000, "limited": bool(limited)}
        data = json.dumps(record, separators=(",", ":")).encode()
        if len(data) > MAX_RECORD:
            raise ValueError("oversized state")
        fd, temporary = tempfile.mkstemp(prefix=".event-", dir=root)
        try:
            with os.fdopen(fd, "wb") as target:
                target.write(data)
                target.flush()
            os.replace(temporary, path)
        finally:
            with contextlib.suppress(FileNotFoundError):
                os.unlink(temporary)
        return record


def alive(pid):
    if type(pid) is not int or pid <= 1:
        return False
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def prune(root):
    records = []
    for index, path in enumerate(root.iterdir()):
        if index >= 256:
            break
        if not re.fullmatch(r"[0-9a-f-]{36}\.json", path.name):
            continue
        try:
            record = read_record(path)
            records.append((record.get("updated_ms", 0), path, record.get("pid")))
        except (OSError, ValueError, TypeError):
            continue
    if len(records) < 16:
        return
    for _, path, pid in sorted(records, key=lambda item: item[0] if type(item[0]) is int else 0):
        if len(records) < 16:
            break
        if alive(pid):
            continue
        with contextlib.suppress(OSError):
            path.unlink()
            path.with_suffix(".lock").unlink(missing_ok=True)
        records.pop()


def input_data():
    data = sys.stdin.buffer.read(MAX_INPUT + 1)
    if len(data) > MAX_INPUT:
        raise ValueError("oversized hook")
    result = json.loads(data or b"{}")
    if not isinstance(result, dict):
        raise ValueError("invalid hook")
    return result


def hook_state(provider, event, data, previous):
    if provider == "opencode" and event in STATES:
        return event, bool(data.get("limited", False))
    if provider == "agy":
        if data.get("tool_confirmation_pending") is True:
            return "permission", False
        agent = data.get("agent_state")
        if agent in {"working", "tool_use", "initializing"}:
            return "working", False
        if agent in {"reviewing", "authenticating"}:
            return "input", False
        if agent == "error":
            return "error", False
        if agent == "idle":
            tasks = data.get("task_count", 0)
            if type(tasks) is int and tasks > 0:
                return "working", False
            old = previous.get("state") if previous else None
            if old == "error":
                return "error", False
            return ("done" if old in {"working", "input", "permission", "done"} else "ready"), False
        return "unknown", True
    event = data.get("hook_event_name", event)
    if event == "SessionStart" or event == "Interrupt":
        return "ready", False
    if event == "SessionEnd":
        if previous and previous.get("state") in {"done", "error"}:
            return previous["state"], bool(previous.get("limited", False))
        return "ready", False
    if event in {"PermissionRequest", "PermissionDenied"}:
        return "permission", False
    if event == "PreToolUse":
        tool = data.get("tool_name")
        if tool in {"AskUserQuestion", "request_user_input"}:
            return "input", False
        if tool == "ExitPlanMode":
            return "permission", False
        return "working", False
    if event == "Stop":
        return ("working" if data.get("background_tasks") else "done"), False
    if event == "StopFailure":
        return "error", False
    if event == "Elicitation":
        return "input", False
    if event in {"UserPromptSubmit", "PreModelRequest", "PostToolUse", "PostToolUseFailure", "ElicitationResult"}:
        return "working", False
    if event == "PostModelRequest":
        return ("error" if data.get("status") == "failed" else "working"), False
    if event == "Notification":
        notification = data.get("notification_type")
        if notification == "permission_prompt":
            return "permission", False
        if notification in {"elicitation_dialog", "elicitation_url_dialog", "agent_needs_input"}:
            return "input", False
        if notification in {"idle_prompt", "agent_completed"}:
            return "done", False
    return (previous.get("state", "unknown"), bool(previous.get("limited", True))) if previous else ("unknown", True)


def hook(provider, event):
    state = "unknown"
    try:
        data = input_data()
        if event in {"SubagentStart", "SubagentStop"}:
            return
        result = update(provider, state, transform=lambda previous: hook_state(provider, event, data, previous))
        state = result["state"]
    except (OSError, ValueError, TypeError, OverflowError):
        pass
    finally:
        if provider == "agy":
            labels = {"working": "Working", "input": "Needs input", "permission": "Needs permission",
                      "done": "Done", "error": "Error", "ready": "Ready", "unknown": "Connected"}
            print("Antigravity · " + labels.get(state, "Connected"))
        else:
            print("{}")


def passthrough(provider, args):
    if any(value in {"--help", "-h", "--version", "-V"} for value in args):
        return True
    if provider == "claude" and "-v" in args:
        return True
    if not args or args[0].startswith("-"):
        return False
    commands = {
        "claude": {"auth", "auto-mode", "doctor", "mcp", "plugin", "plugins", "install", "update", "upgrade", "import", "purge", "daemon", "agents", "logs", "stop", "kill", "rm", "setup-token", "gateway", "respawn"},
        "codex": {"login", "logout", "mcp", "mcp-server", "app-server", "completion", "debug", "apply", "features", "sandbox", "cloud", "agents", "plugin", "remote-control", "update", "doctor"},
        "opencode": {"auth", "models", "debug", "serve", "upgrade", "uninstall", "mcp", "export", "import", "stats", "completion", "acp"},
        "agy": {"auth", "login", "logout", "mcp", "install", "update", "upgrade", "doctor", "version"},
    }
    return args[0] in commands[provider]


def real_command(provider, fallback=True):
    root = Path(os.environ.get("TERMUST_AGENT_DIR", "."))
    sources = [os.environ.get("PATH", "")]
    if fallback:
        sources.append(os.environ.get("TERMUST_AGENT_ORIGINAL_PATH", ""))
    for source in sources:
        entries = []
        for value in source.split(os.pathsep):
            directory = Path(os.path.realpath(value or "."))
            if directory == root / "bin" or (directory.name in {"bin", "proxies"} and (directory.parent / "agent_bridge.py").is_file()):
                continue
            entries.append(value)
        result = shutil.which(provider, path=os.pathsep.join(entries))
        if result:
            return result
    raise FileNotFoundError(provider)


def setup():
    root = private_root()
    for directory in (root / "bin", root / "proxies"):
        info = directory.lstat()
        if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o022:
            raise ValueError("unsafe proxy directory")
    for provider in PROVIDERS:
        try:
            real_command(provider, fallback=False)
            source = root / "proxies" / provider
            fd = safe_open(source, os.O_RDONLY)
            executable = os.fstat(fd).st_mode & 0o111
            os.close(fd)
            if not executable:
                continue
            target = root / "bin" / provider
            relative = "../proxies/" + provider
            if target.exists() or target.is_symlink():
                if not target.is_symlink() or os.readlink(target) != relative:
                    continue
            else:
                os.symlink(relative, target)
        except (OSError, ValueError):
            continue


def settings_value(value):
    if value.lstrip().startswith("{"):
        data = value.encode()
    else:
        with open(value, "rb") as source:
            data = source.read(MAX_INPUT + 1)
    if len(data) > MAX_INPUT:
        raise ValueError("oversized settings")
    result = json.loads(data)
    if not isinstance(result, dict):
        raise ValueError("invalid settings")
    return result


def explicit_settings(args):
    output, values = [], []
    iterator = iter(args)
    for value in iterator:
        if value == "--settings":
            values.append(next(iterator))
        elif value.startswith("--settings="):
            values.append(value.partition("=")[2])
        else:
            output.append(value)
    return output, settings_value(values[-1]) if values else {}


def private_settings(root, identity, value):
    directory = root / "settings"
    try:
        directory.mkdir(mode=0o700)
    except FileExistsError:
        info = directory.lstat()
        if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
            raise ValueError("unsafe settings directory")
    path = directory / (identity + ".json")
    fd = safe_open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL)
    with os.fdopen(fd, "w") as target:
        json.dump(value, target, separators=(",", ":"))
    return path


def opencode_plugin(root):
    directory = root / "opencode-plugin"
    try:
        directory.mkdir(mode=0o700)
    except FileExistsError:
        info = directory.lstat()
        if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
            raise ValueError("unsafe plugin directory")
    source = root / "agent_opencode.ts"
    fd = safe_open(source, os.O_RDONLY)
    with os.fdopen(fd, "rb") as handle:
        content = handle.read(MAX_INPUT + 1)
    if len(content) > MAX_INPUT:
        raise ValueError("oversized plugin")
    package = {"name": "termust-agent-status", "type": "module", "exports": {"./tui": "./tui.ts"}}
    for name, data in (("tui.ts", content), ("package.json", json.dumps(package).encode())):
        target = directory / name
        if target.exists() or target.is_symlink():
            os.close(safe_open(target, os.O_RDONLY))
        fd, temporary = tempfile.mkstemp(prefix=".plugin-", dir=directory)
        try:
            with os.fdopen(fd, "wb") as handle:
                handle.write(data)
            os.replace(temporary, target)
        finally:
            with contextlib.suppress(FileNotFoundError):
                os.unlink(temporary)
    return str(directory)


def prepare(provider, args, env, root, identity):
    bridge = str(Path(__file__).absolute())
    env["TERMUST_AGENT_BRIDGE"] = bridge
    env["TERMUST_AGENT_PYTHON"] = sys.executable
    command = '"$TERMUST_AGENT_PYTHON" "$TERMUST_AGENT_BRIDGE" hook ' + provider
    if provider == "codex":
        return args, None
    if provider == "opencode":
        plugin = opencode_plugin(root)
        config = settings_value(env.get("OPENCODE_CLI_CONFIG_CONTENT", "{}"))
        if "plugins" not in config:
            home = Path(env.get("XDG_CONFIG_HOME") or (Path.home() / ".config"))
            global_path = home / "opencode/cli.json"
            config["plugins"] = settings_value(str(global_path)).get("plugins", []) if global_path.exists() else []
        plugins = config["plugins"]
        if not isinstance(plugins, list):
            raise ValueError("invalid plugins")
        if plugin not in plugins:
            plugins.append(plugin)
        legacy = settings_value(env.get("OPENCODE_TUI_CONFIG", "{}"))
        legacy_plugins = legacy.setdefault("plugin", [])
        if not isinstance(legacy_plugins, list):
            raise ValueError("invalid legacy plugins")
        legacy_plugin = str(root / "agent_opencode.ts")
        if legacy_plugin not in legacy_plugins:
            legacy_plugins.append(legacy_plugin)
        env["OPENCODE_CLI_CONFIG_CONTENT"] = json.dumps(config, separators=(",", ":"))
        env["OPENCODE_TUI_CONFIG"] = json.dumps(legacy, separators=(",", ":"))
        return args, None
    args, config = explicit_settings(args)
    if provider == "claude":
        hooks = config.setdefault("hooks", {})
        if not isinstance(hooks, dict):
            raise ValueError("invalid hooks")
        for event in CLAUDE_EVENTS:
            entries = hooks.setdefault(event, [])
            if not isinstance(entries, list):
                raise ValueError("invalid hook list")
            entries.append({"hooks": [{"type": "command", "command": command + " " + event, "timeout": 2}]})
    else:
        config["title"] = {"type": "command", "command": command + " title", "enabled": True}
    path = private_settings(root, identity, config)
    return ["--settings", str(path)] + args, path


def set_foreground(fd, group):
    previous = signal.signal(signal.SIGTTOU, signal.SIG_IGN)
    try:
        os.tcsetpgrp(fd, group)
    finally:
        signal.signal(signal.SIGTTOU, previous)


def launch(command, env):
    tty = None
    try:
        if os.isatty(0) and os.tcgetpgrp(0) == os.getpgrp():
            tty = (0, os.getpgrp())
    except OSError:
        pass
    child = subprocess.Popen(command, env=env, preexec_fn=os.setpgrp if tty else None)
    previous = {}
    def forward(number, frame):
        with contextlib.suppress(ProcessLookupError):
            if tty:
                os.killpg(child.pid, number)
            else:
                child.send_signal(number)
    for number in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        previous[number] = signal.signal(number, forward)
    try:
        if tty:
            set_foreground(tty[0], child.pid)
            os.killpg(child.pid, signal.SIGCONT)
        return child.wait()
    finally:
        if tty:
            with contextlib.suppress(OSError):
                set_foreground(*tty)
        for number, handler in previous.items():
            signal.signal(number, handler)


def run(provider, args):
    if provider not in PROVIDERS:
        return 2
    try:
        real = real_command(provider)
    except (OSError, ValueError):
        print("Termust: command not found: " + provider, file=sys.stderr)
        return 127
    if passthrough(provider, args):
        os.execv(real, [real] + args)
    env = dict(os.environ)
    identity = str(uuid.uuid4())
    env["TERMUST_AGENT_RUN"] = identity
    env["TERMUST_AGENT_PID"] = str(os.getpid())
    os.environ["TERMUST_AGENT_RUN"] = identity
    os.environ["TERMUST_AGENT_PID"] = str(os.getpid())
    path = None
    try:
        root = private_root()
        prune(root)
        update(provider, "unknown", limited=True)
        forwarded, path = prepare(provider, args, env, root, identity)
    except (OSError, ValueError, TypeError, StopIteration):
        forwarded = args
    result = 1
    try:
        result = launch([real] + forwarded, env)
        return result if result >= 0 else 128 - result
    except OSError:
        return 126
    finally:
        try:
            previous = read_record(private_root() / (identity + ".json"))
            limited = bool(previous.get("limited", True)) if previous else True
            interrupted = {signal.SIGINT, signal.SIGTERM, signal.SIGHUP}
            state = "ready" if any(result in {-number, 128 + number} for number in interrupted) else ("done" if result == 0 else "error")
            if result == 0 and previous and previous.get("state") == "error":
                state = "error"
            update(provider, state, limited=limited)
        except (OSError, ValueError, TypeError):
            pass
        if path is not None:
            with contextlib.suppress(OSError):
                path.unlink()


def main():
    if len(sys.argv) == 2 and sys.argv[1] == "setup":
        try:
            setup()
        except (OSError, ValueError):
            pass
        return 0
    if len(sys.argv) < 3:
        return 2
    operation, provider = sys.argv[1:3]
    if operation == "hook":
        hook(provider, sys.argv[3] if len(sys.argv) > 3 else "")
        return 0
    if operation == "run":
        return run(provider, sys.argv[3:])
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
