import contextlib
import importlib.util
import json
import os
from pathlib import Path
import pty
import signal
import subprocess
import sys
import tempfile
import time
import unittest
import uuid
from unittest.mock import patch


SOURCE = Path(__file__).resolve().parents[1] / "alacritty/src/agent_bridge.py"
SPEC = importlib.util.spec_from_file_location("termust_agent_bridge", SOURCE)
bridge = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bridge)

FAKE = r'''import json,os,signal,subprocess,sys,time
from pathlib import Path
args=sys.argv[1:]
capture={"argv":args,"run":os.environ.get("TERMUST_AGENT_RUN")}
settings={}
if "--settings" in args:
    value=args[args.index("--settings")+1]
    try:settings=json.loads(Path(value).read_text())
    except Exception:pass
capture["settings"]=settings
capture["opencode_config"]=os.environ.get("OPENCODE_TUI_CONFIG")
capture["opencode_cli_config"]=os.environ.get("OPENCODE_CLI_CONFIG_CONTENT")
capture["tty"]=os.isatty(0)
capture["foreground"]=os.tcgetpgrp(0)==os.getpgrp() if os.isatty(0) else False
Path(os.environ["TEST_CAPTURE"]).write_text(json.dumps(capture))
mode=os.environ.get("TEST_MODE","")
if mode in {"sleep","interrupt"}:
    Path(os.environ["TEST_STARTED"]).write_text(str(os.getpid()))
    if mode=="interrupt":
        def interrupt(*args):
            Path(os.environ["TEST_SIGNALS"]).write_text("one")
            sys.exit(0)
        signal.signal(signal.SIGINT,interrupt)
    while True:time.sleep(.05)
events=json.loads(os.environ.get("TEST_EVENTS","[]"))
for event,payload in events:
    hooks=settings.get("hooks",{}).get(event,[])
    for item in hooks:
        for hook in item.get("hooks",[]):
            subprocess.run(hook["command"],shell=True,input=json.dumps(dict(payload,hook_event_name=event)).encode(),stdout=subprocess.PIPE,check=True)
if mode=="agy":
    for payload in events:
        subprocess.run(settings["title"]["command"],shell=True,input=json.dumps(payload[1]).encode(),stdout=subprocess.PIPE,check=True)
print("fake-agent-output")
sys.exit(int(os.environ.get("TEST_EXIT","0")))
'''


def wait_for(predicate, seconds=5):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(.02)
    raise AssertionError("Timed out waiting for isolated fake harness")


class BridgeTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="termust-agent-test-")
        self.base = Path(self.directory.name)
        self.root = self.base / "private"
        self.root.mkdir(mode=0o700)
        (self.root / "bin").mkdir(mode=0o700)
        plugin = self.root / "agent_opencode.ts"
        plugin.write_text("export default {}")
        plugin.chmod(0o600)
        self.bin = self.base / "real"
        self.bin.mkdir()
        self.capture = self.base / "capture.json"
        self.started = self.base / "started"
        self.signals = self.base / "signals"
        self.identity = str(uuid.uuid4())
        self.env = dict(os.environ, TERMUST_AGENT_DIR=str(self.root),
                        TERMUST_AGENT_RUN=self.identity, TERMUST_AGENT_PID=str(os.getpid()),
                        TERMUST_AGENT_ORIGINAL_PATH=str(self.bin) + os.pathsep + os.environ["PATH"],
                        TEST_CAPTURE=str(self.capture), TEST_STARTED=str(self.started),
                        TEST_SIGNALS=str(self.signals), TEST_EVENTS="[]", TEST_MODE="", TEST_EXIT="0")
        self.env["XDG_CONFIG_HOME"] = str(self.base / "config")
        self.env["PATH"] = str(self.root / "bin") + os.pathsep + self.env["TERMUST_AGENT_ORIGINAL_PATH"]
        for provider in bridge.PROVIDERS:
            path = self.bin / provider
            path.write_text("#!" + sys.executable + "\n" + FAKE)
            path.chmod(0o700)

    def tearDown(self):
        self.directory.cleanup()

    def invoke(self, operation, provider="claude", args=(), data=None, env=None):
        return subprocess.run([sys.executable, str(SOURCE), operation, provider] + list(args),
                              env=env or self.env, input=json.dumps(data or {}).encode(),
                              capture_output=True, timeout=10)

    def record(self, identity=None):
        return json.loads((self.root / ((identity or self.identity) + ".json")).read_text())

    def only_record(self):
        paths = list(self.root.glob("*.json"))
        self.assertEqual(len(paths), 1)
        return json.loads(paths[0].read_text())

    def test_hook_states_and_metadata_only_records(self):
        cases = [("SessionStart", {}, "ready"), ("UserPromptSubmit", {}, "working"),
                 ("PreToolUse", {"tool_name": "AskUserQuestion"}, "input"),
                 ("PreToolUse", {"tool_name": "ExitPlanMode"}, "permission"),
                 ("PermissionRequest", {}, "permission"), ("PostToolUse", {}, "working"),
                 ("Stop", {"background_tasks": [{"description": "private"}]}, "working"),
                 ("Stop", {}, "done"), ("StopFailure", {"error": "rate_limit"}, "error")]
        for index, (event, payload, state) in enumerate(cases, 1):
            payload.update(prompt="SENSITIVE_PROMPT", transcript_path="PRIVATE_TRANSCRIPT", tool_input={"secret": "SECRET"})
            result = self.invoke("hook", args=[event], data=payload)
            self.assertEqual(result.returncode, 0)
            self.assertEqual(result.stdout.strip(), b"{}")
            record = self.record()
            self.assertEqual(record["state"], state)
            self.assertEqual(record["sequence"], index)
            self.assertFalse(record["limited"])
            self.assertEqual(set(record), {"v", "provider", "state", "run", "pid", "sequence", "updated_ms", "limited"})
            self.assertEqual((self.root / (self.identity + ".json")).stat().st_mode & 0o777, 0o600)
            self.assertNotIn("SENSITIVE", json.dumps(record))
            self.assertNotIn("SECRET", json.dumps(record))

    def test_question_notification_and_failure_do_not_make_tool_errors_terminal(self):
        for event, data, expected in [
            ("Notification", {"notification_type": "permission_prompt"}, "permission"),
            ("Elicitation", {}, "input"), ("ElicitationResult", {}, "working"),
            ("PostToolUseFailure", {}, "working"), ("Stop", {}, "done"),
            ("SessionEnd", {}, "done")]:
            self.invoke("hook", args=[event], data=data)
            self.assertEqual(self.record()["state"], expected)
        self.invoke("hook", args=["SubagentStop"], data={"agent_id": "child"})
        self.assertEqual(self.record()["state"], "done")

    def test_codex_main_events_with_agent_id_are_not_discarded(self):
        result = self.invoke("hook", "codex", ["PreModelRequest"], {"agent_id": "root", "agent_type": "default"})
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), b"{}")
        self.assertEqual(self.record()["state"], "working")

    def test_unsafe_and_oversized_hook_inputs_fail_open_without_changing_records(self):
        self.invoke("hook", args=["UserPromptSubmit"])
        path = self.root / (self.identity + ".json")
        before = path.read_bytes()
        result = subprocess.run([sys.executable, str(SOURCE), "hook", "claude", "Stop"],
                                env=self.env, input=b"{" + b"x" * bridge.MAX_INPUT,
                                capture_output=True, timeout=5)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(path.read_bytes(), before)
        path.chmod(0o644)
        self.invoke("hook", args=["Stop"])
        self.assertEqual(path.read_bytes(), before)
        path.unlink()
        victim = self.base / "victim"
        victim.write_text("unchanged")
        path.symlink_to(victim)
        self.invoke("hook", args=["Stop"])
        self.assertEqual(victim.read_text(), "unchanged")
        path.unlink()
        os.link(victim, path)
        self.invoke("hook", args=["Stop"])
        self.assertEqual(victim.read_text(), "unchanged")

    def test_concurrent_hook_writes_preserve_monotonic_sequence(self):
        processes = [subprocess.Popen([sys.executable, str(SOURCE), "hook", "claude", "UserPromptSubmit"],
                                      env=self.env, stdin=subprocess.PIPE, stdout=subprocess.PIPE)
                     for _ in range(20)]
        for process in processes:
            process.communicate(b"{}", timeout=5)
            self.assertEqual(process.returncode, 0)
        self.assertEqual(self.record()["sequence"], 20)

    def test_actual_fake_cli_preserves_arguments_user_settings_and_hook_chain(self):
        settings = self.base / "user settings.json"
        original = {"theme": "light", "permissions": {"defaultMode": "manual"},
                    "hooks": {"UserPromptSubmit": [{"hooks": [{"type": "command", "command": "true"}]}]}}
        settings.write_text(json.dumps(original))
        env = dict(self.env, TEST_EVENTS=json.dumps([["SessionStart", {}], ["UserPromptSubmit", {}], ["Stop", {}]]))
        result = self.invoke("run", args=["--settings", str(settings), "--permission-mode", "manual", "literal prompt $()"], env=env)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), b"fake-agent-output")
        captured = json.loads(self.capture.read_text())
        self.assertEqual(captured["settings"]["theme"], "light")
        self.assertEqual(captured["settings"]["permissions"], original["permissions"])
        self.assertEqual(captured["settings"]["hooks"]["UserPromptSubmit"][0], original["hooks"]["UserPromptSubmit"][0])
        self.assertEqual(captured["argv"][2:], ["--permission-mode", "manual", "literal prompt $()"])
        self.assertNotIn("--dangerously-skip-permissions", captured["argv"])
        self.assertEqual(json.loads(settings.read_text()), original)
        record = self.only_record()
        self.assertEqual(record["state"], "done")
        self.assertFalse(record["limited"])
        self.assertEqual(record["sequence"], 5)
        self.assertFalse(list((self.root / "settings").glob("*.json")))

    def test_help_version_and_maintenance_subcommands_are_exact_passthrough(self):
        for provider, args in [("claude", ["--help"]), ("codex", ["--version"]),
                               ("agy", ["--version"]), ("opencode", ["models"]), ("claude", ["auth", "status"])]:
            self.env.pop("TERMUST_AGENT_RUN", None)
            result = self.invoke("run", provider, args)
            self.assertEqual(result.returncode, 0)
            captured = json.loads(self.capture.read_text())
            self.assertEqual(captured["argv"], args)
            self.assertIsNone(captured["run"])
            self.assertFalse(list(self.root.glob("*.json")))

    def test_invalid_explicit_settings_preserves_original_cli_behavior(self):
        args = ["--settings", str(self.base / "missing.json"), "hello"]
        result = self.invoke("run", args=args)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(json.loads(self.capture.read_text())["argv"], args)
        self.assertTrue(self.only_record()["limited"])

    def test_codex_native_status_keeps_cli_arguments_and_trust_unchanged(self):
        args = ["--config", "model='custom'", "--no-alt-screen"]
        result = self.invoke("run", "codex", args)
        self.assertEqual(result.returncode, 0, result.stderr)
        captured = json.loads(self.capture.read_text())
        self.assertEqual(captured["argv"], args)
        self.assertNotIn("--dangerously-bypass-approvals-and-sandbox", captured["argv"])
        self.assertNotIn("--no-daemon", captured["argv"])
        self.assertEqual(self.only_record()["state"], "done")
        self.assertTrue(self.only_record()["limited"])

    def test_explicit_codex_hooks_are_not_overwritten(self):
        args = ["-c", "hooks.Stop=[]"]
        result = self.invoke("run", "codex", args)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(json.loads(self.capture.read_text())["argv"], args)
        self.assertTrue(self.only_record()["limited"])

    def test_codex_remote_and_older_harnesses_stay_usable_with_limited_status(self):
        for args, env in [(["--remote", "unix:///tmp/server"], self.env),
                          (["hello"], dict(self.env, TEST_CODEX_OLD="1"))]:
            result = self.invoke("run", "codex", args, env=env)
            self.assertEqual(result.returncode, 0)
            self.assertEqual(json.loads(self.capture.read_text())["argv"], args)
        for path in self.root.glob("*.json"):
            self.assertTrue(json.loads(path.read_text())["limited"])

    def test_current_shell_path_takes_precedence_over_original_path(self):
        custom = self.base / "custom"
        custom.mkdir()
        script = custom / "claude"
        script.write_text("#!/bin/sh\nprintf custom-harness")
        script.chmod(0o700)
        env = dict(self.env, PATH=str(self.root / "bin") + os.pathsep + str(custom) + os.pathsep + self.env["PATH"])
        result = self.invoke("run", args=["--version"], env=env)
        self.assertEqual(result.stdout, b"custom-harness")

    def proxy_templates(self):
        directory = self.root / "proxies"
        directory.mkdir(mode=0o700)
        for provider in bridge.PROVIDERS:
            target = directory / provider
            target.write_text("#!/bin/sh\nexit 0\n")
            target.chmod(0o700)

    def invoke_setup(self, path):
        return subprocess.run([sys.executable, str(SOURCE), "setup"],
                              env=dict(self.env, PATH=path), capture_output=True, timeout=5)

    def test_setup_does_not_advertise_missing_harnesses(self):
        self.proxy_templates()
        empty = self.base / "empty-path"
        empty.mkdir()
        result = self.invoke_setup(str(self.root / "bin") + os.pathsep + str(empty))
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout, b"")
        self.assertEqual(list((self.root / "bin").iterdir()), [])
        self.assertFalse(list(self.root.glob("*.json")))

    def test_setup_discovers_shell_rc_paths_without_overwriting_existing_files(self):
        self.proxy_templates()
        added = self.base / "shell-rc-bin"
        added.mkdir()
        target = added / "claude"
        target.write_text("#!/bin/sh\nexit 0\n")
        target.chmod(0o700)
        result = self.invoke_setup(str(added))
        self.assertEqual(result.returncode, 0)
        proxy = self.root / "bin/claude"
        self.assertTrue(proxy.is_symlink())
        self.assertEqual(os.readlink(proxy), "../proxies/claude")
        self.assertEqual(len(list((self.root / "bin").iterdir())), 1)
        proxy.unlink()
        victim = self.base / "untouched"
        victim.write_text("unchanged")
        proxy.symlink_to(victim)
        self.invoke_setup(str(added))
        self.assertEqual(os.readlink(proxy), str(victim))
        self.assertEqual(victim.read_text(), "unchanged")
        self.assertFalse(list(self.root.glob("*.json")))

    def test_opencode_preserves_effective_plugins_and_other_inline_settings(self):
        directory = self.base / "config/opencode"
        directory.mkdir(parents=True)
        source = directory / "cli.json"
        original = {"plugins": ["global-plugin"], "theme": "global-theme"}
        source.write_text(json.dumps(original))
        env = dict(self.env, OPENCODE_CLI_CONFIG_CONTENT='{"theme":"inline-theme"}',
                   OPENCODE_TUI_CONFIG='{"plugin":["legacy-plugin"],"theme":"legacy-theme"}')
        self.assertEqual(self.invoke("run", "opencode", env=env).returncode, 0)
        captured = json.loads(self.capture.read_text())
        config = json.loads(captured["opencode_cli_config"])
        self.assertEqual(config["theme"], "inline-theme")
        self.assertEqual(config["plugins"], ["global-plugin", str(self.root / "opencode-plugin")])
        legacy = json.loads(captured["opencode_config"])
        self.assertEqual(legacy["plugin"], ["legacy-plugin", str(self.root / "agent_opencode.ts")])
        self.assertEqual(json.loads(source.read_text()), original)
        package = json.loads((self.root / "opencode-plugin/package.json").read_text())
        self.assertEqual(package["exports"], {"./tui": "./tui.ts"})
        self.assertEqual((self.root / "opencode-plugin/tui.ts").read_text(), "export default {}")
        env["OPENCODE_CLI_CONFIG_CONTENT"] = '{"plugins":["explicit-override"]}'
        self.assertEqual(self.invoke("run", "opencode", env=env).returncode, 0)
        config = json.loads(json.loads(self.capture.read_text())["opencode_cli_config"])
        self.assertEqual(config["plugins"], ["explicit-override", str(self.root / "opencode-plugin")])

    def test_agy_title_maps_live_metadata_without_recording_content(self):
        payloads = [{"agent_state": "working", "user_email": "private@example.test"},
                    {"agent_state": "idle", "tool_confirmation_pending": True},
                    {"agent_state": "reviewing"}, {"agent_state": "idle", "task_count": 3},
                    {"agent_state": "idle", "task_count": 0}]
        for payload, expected in zip(payloads, ["working", "permission", "input", "working", "done"]):
            result = self.invoke("hook", "agy", ["title"], payload)
            self.assertTrue(result.stdout.startswith(b"Antigravity"))
            self.assertNotIn(b"private@example.test", result.stdout)
            self.assertEqual(self.record()["state"], expected)
        result = self.invoke("run", "agy", ["--settings", '{"theme":"light"}'],
                             env=dict(self.env, TEST_MODE="agy", TEST_EVENTS=json.dumps([["title", {"agent_state": "working"}]])))
        self.assertEqual(result.returncode, 0)
        self.assertEqual(json.loads(self.capture.read_text())["settings"]["theme"], "light")

    def test_normal_cli_exit_does_not_erase_last_reported_turn_error(self):
        env = dict(self.env, TEST_EVENTS=json.dumps([["UserPromptSubmit", {}], ["StopFailure", {"error": "rate_limit"}]]))
        result = self.invoke("run", env=env)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(self.only_record()["state"], "error")
        self.assertFalse(self.only_record()["limited"])

    def test_agy_error_survives_idle_until_new_work(self):
        for payload, expected in [({"agent_state": "error"}, "error"),
                                  ({"agent_state": "idle", "task_count": 0}, "error"),
                                  ({"agent_state": "working"}, "working"),
                                  ({"agent_state": "idle", "task_count": 0}, "done")]:
            self.assertEqual(self.invoke("hook", "agy", ["title"], payload).returncode, 0)
            self.assertEqual(self.record()["state"], expected)

    def test_prunes_old_dead_runs_without_removing_live_status(self):
        with patch.dict(os.environ, self.env, clear=True):
            bridge.update("claude", "working")
        for index in range(20):
            identity = str(uuid.uuid4())
            path = self.root / (identity + ".json")
            path.write_text(json.dumps({"pid": 999999999, "updated_ms": index}))
            path.chmod(0o600)
        with patch.dict(os.environ, self.env, clear=True):
            bridge.prune(self.root)
        self.assertLess(len(list(self.root.glob("*.json"))), 16)
        self.assertEqual(self.record()["state"], "working")

    def test_wrapper_forwards_sigterm_and_preserves_signal_exit_code(self):
        process = subprocess.Popen([sys.executable, str(SOURCE), "run", "claude"],
                                   env=dict(self.env, TEST_MODE="sleep"), stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            pid = int(wait_for(lambda: self.started.read_text() if self.started.exists() else None))
            process.send_signal(signal.SIGTERM)
            process.communicate(timeout=5)
            self.assertEqual(process.returncode, 143)
            self.assertEqual(self.only_record()["state"], "ready")
            with self.assertRaises(ProcessLookupError):
                os.kill(pid, 0)
        finally:
            if process.poll() is None:
                process.kill()
                process.communicate()

    def test_pty_handoff_delivers_ctrl_c_once_to_real_harness(self):
        pid, fd = pty.fork()
        if pid == 0:
            os.execve(sys.executable, [sys.executable, str(SOURCE), "run", "claude"], dict(self.env, TEST_MODE="interrupt"))
        try:
            wait_for(lambda: self.started.exists())
            captured = json.loads(self.capture.read_text())
            self.assertTrue(captured["tty"])
            self.assertTrue(captured["foreground"])
            os.write(fd, b"\x03")
            wait_for(lambda: self.signals.exists())
            _, status = os.waitpid(pid, 0)
            self.assertEqual(os.waitstatus_to_exitcode(status), 0)
            self.assertEqual(self.signals.read_text(), "one")
        finally:
            os.close(fd)
            with contextlib.suppress(ProcessLookupError):
                os.kill(pid, signal.SIGKILL)
            with contextlib.suppress(ChildProcessError):
                os.waitpid(pid, 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
