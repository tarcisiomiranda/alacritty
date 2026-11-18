#!/usr/bin/env python3

import configparser
import fcntl
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
INSTALLER = ROOT / "install.sh"
VERSION = "0.18.0-termust-test"
BINARY = b'#!/usr/bin/env bash\nprintf "termust 0.18.0 (fixture)\\n"\n'
DESKTOP = """[Desktop Entry]
Type=Application
Name=Termust
Exec=termust
TryExec=termust
Icon=Termust
Actions=new-window;

[Desktop Action new-window]
Name=New Window
Name[pt_BR]=Nova janela
Exec=termust
"""


class InstallerTests(unittest.TestCase):
    def test_personal_icon_survives_reinstall_without_losing_launcher_updates(self):
        first = self.run_installer()
        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        launcher = self.prefix / "share/applications/Termust.desktop"
        icon = self.directory / "user data/cat.png"
        icon.parent.mkdir()
        icon.write_bytes(b"personal icon fixture")
        text = launcher.read_text().replace("Icon=Termust\n",
            f"Icon={icon}\nX-Termust-IconPreset=cat\nX-Termust-OriginalIcon=Termust\n")
        launcher.write_text(text.replace("Name=Termust", "Name=Old name"))
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        text = launcher.read_text()
        self.assertIn(f"Icon={icon}\n", text)
        self.assertIn("X-Termust-IconPreset=cat\n", text)
        self.assertIn("X-Termust-OriginalIcon=Termust\n", text)
        self.assertIn("Name=Termust\n", text)
        self.assertIn("[Desktop Action new-window]", text)

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="termust-install-test-")
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.prefix = self.directory / "prefix"
        self.archive = self.directory / "termust-linux-x86_64.tar.gz"
        self.environment = dict(os.environ)
        self.environment["HOME"] = str(self.directory / "home")
        self.environment["TMPDIR"] = str(self.directory / "temporary")
        Path(self.environment["HOME"]).mkdir()
        Path(self.environment["TMPDIR"]).mkdir()
        self.make_archive()

    def make_archive(self, binary=BINARY, extra=None, version=VERSION, terminfo=b"alacritty|fixture terminal, am,\n", metadata=None):
        entries = {
            "bin/termust": binary,
            "VERSION": (version + "\n").encode(),
            "share/applications/Termust.desktop": DESKTOP.encode(),
            "share/icons/hicolor/scalable/apps/Termust.svg": b"<svg/>",
            "share/terminfo/alacritty.info": terminfo,
            "share/bash-completion/completions/termust": b"complete -W '' termust\n",
            "share/fish/vendor_completions.d/termust.fish": b"complete -c termust\n",
            "share/zsh/site-functions/_termust": b"_arguments\n",
            "share/doc/termust/LICENSE-APACHE": b"fixture license\n",
            "share/doc/termust/README.md": b"fixture documentation\n",
        }
        if metadata is not None:
            entries["release.json"] = (json.dumps(metadata) + "\n").encode()
        with tarfile.open(self.archive, "w:gz") as package:
            for name, data in entries.items():
                member = tarfile.TarInfo("termust/" + name)
                member.mode = 0o755 if name == "bin/termust" else 0o644
                member.size = len(data)
                package.addfile(member, io.BytesIO(data))
            if extra:
                for member, data in extra:
                    package.addfile(member, io.BytesIO(data) if data is not None else None)
        checksum = hashlib.sha256(self.archive.read_bytes()).hexdigest()
        Path(str(self.archive) + ".sha256").write_text(checksum + "  " + self.archive.name + "\n")

    def run_installer(self, *arguments, remote=False, source=None):
        command = ["bash", str(INSTALLER)]
        if source is not None:
            command = ["bash", "-s", "--"]
        command += ["--prefix", str(self.prefix)]
        if not remote and not any(option in arguments for option in ("--check", "--rollback")):
            command += ["--archive", str(self.archive)]
        result = subprocess.run(
            command + list(arguments),
            input=source,
            text=True,
            capture_output=True,
            env=self.environment,
            cwd=self.directory,
            timeout=30,
        )
        self.assertEqual(list(Path(self.environment["TMPDIR"]).iterdir()), [])
        return result

    def old_install(self):
        binary = self.prefix / "bin/termust"
        binary.parent.mkdir(parents=True, exist_ok=True)
        binary.write_bytes(b"previous terminal\n")
        binary.chmod(0o755)
        return binary

    def managed_alacritty(self):
        version = "0.18.0-workspace.c14a29f6.66afacc21311"
        binary = self.prefix / "bin/alacritty"
        binary.parent.mkdir(parents=True, exist_ok=True)
        binary.write_bytes(b'#!/usr/bin/env bash\nif [[ "${1:-}" == --version ]]; then\n'
            b'printf "alacritty 0.18.0-dev (c14a29f6)\\n"\nelse\nexec sleep 30\nfi\n')
        binary.chmod(0o755)
        executable = str(binary)
        escaped = executable.replace("\\", "\\\\").replace('"', '\\"').replace("`", "\\`").replace("$", "\\$")
        command = ('"' + escaped.replace("%", "%%") + '"').replace("\\", "\\\\")
        launcher = self.prefix / "share/applications/Alacritty.desktop"
        launcher.parent.mkdir(parents=True, exist_ok=True)
        launcher.write_text(f"[Desktop Entry]\nType=Application\nName=Alacritty Workspace\n"
            f"Icon=Alacritty\nTryExec={executable.replace(chr(92), chr(92) * 2)}\nExec={command}\n"
            "Terminal=false\nCategories=System;TerminalEmulator;\nGenericName=Terminal\n"
            "Comment=A fast, cross-platform, OpenGL terminal emulator\nStartupNotify=true\n"
            f"StartupWMClass=Alacritty\nActions=New;\n\n[Desktop Action New]\nName=New Terminal\nExec={command}\n")
        receipt = self.prefix / "share/doc/alacritty-workspace/VERSION"
        receipt.parent.mkdir(parents=True, exist_ok=True)
        receipt.write_text(version + "\n")
        return binary, launcher, receipt

    def assert_preserved(self, result, binary):
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(binary.read_bytes(), b"previous terminal\n")
        self.assertEqual(list(binary.parent.glob(".termust-install.*")), [])

    def fake_remote(self, pointer=VERSION):
        remote = self.directory / "remote"
        release = remote / "releases" / VERSION
        release.mkdir(parents=True)
        shutil.copy2(self.archive, release / self.archive.name)
        shutil.copy2(str(self.archive) + ".sha256", release / (self.archive.name + ".sha256"))
        (remote / "latest-linux-x86_64.txt").write_text(pointer + "\n")
        commands = self.directory / "commands"
        commands.mkdir()
        curl = commands / "curl"
        curl.write_text(
            "#!" + sys.executable + "\n"
            "import os, pathlib, shutil, sys\n"
            "from urllib.parse import urlsplit\n"
            "arguments = sys.argv[1:]\n"
            "assert arguments[arguments.index('--proto') + 1] == '=https'\n"
            "assert arguments[arguments.index('--proto-redir') + 1] == '=https'\n"
            "url = arguments[-1]\n"
            "assert url.startswith('https://downloads.example.test/file/bucket/')\n"
            "relative = urlsplit(url).path.removeprefix('/file/bucket/')\n"
            "with open(os.environ['FAKE_CURL_LOG'], 'a') as log: log.write(relative + '\\n')\n"
            "shutil.copyfile(pathlib.Path(os.environ['FAKE_REMOTE']) / relative, arguments[arguments.index('--output') + 1])\n"
        )
        curl.chmod(0o755)
        self.environment["PATH"] = str(commands) + os.pathsep + self.environment["PATH"]
        self.environment["FAKE_REMOTE"] = str(remote)
        self.environment["FAKE_CURL_LOG"] = str(self.directory / "requests")
        return ["--base-url", "https://downloads.example.test/file/bucket/"]

    def test_local_install_and_integrations(self):
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((self.prefix / "bin/termust").read_bytes(), BINARY)
        self.assertIn(VERSION, result.stdout)
        self.assertEqual((self.prefix / "share/doc/termust/VERSION").read_text(), VERSION + "\n")
        self.assertIn(str(self.prefix / "bin/termust"), result.stdout)
        launcher = (self.prefix / "share/applications/Termust.desktop").read_text()
        self.assertIn("Name=Termust\n", launcher)
        self.assertIn('Exec="' + str(self.prefix / "bin/termust") + '"\n', launcher)
        self.assertEqual(launcher.count('Exec="'), 2)
        self.assertIn("TryExec=" + str(self.prefix / "bin/termust") + "\n", launcher)
        for relative in (
            "icons/hicolor/scalable/apps/Termust.svg",
            "bash-completion/completions/termust",
            "fish/vendor_completions.d/termust.fish",
            "zsh/site-functions/_termust",
            "doc/termust/LICENSE-APACHE",
        ):
            self.assertTrue((self.prefix / "share" / relative).is_file(), relative)

    @unittest.skipUnless(shutil.which("tic") and shutil.which("infocmp"), "ncurses tools unavailable")
    def test_user_terminfo_is_discoverable_and_preserves_other_entries(self):
        user_terminfo = Path(self.environment["HOME"]) / ".terminfo"
        retained = self.directory / "retained.info"
        retained.write_text("alacritty-installer-retained|retained terminal, cols#79,\n")
        subprocess.run(["tic", "-o", str(user_terminfo), str(retained)], check=True)
        before = {path: path.read_bytes() for path in user_terminfo.rglob("*") if path.is_file()}
        self.make_archive(terminfo=(
            b"alacritty|alacritty-installer-probe|fixture terminal, cols#137,\n"
            b"alacritty-direct|alacritty-installer-direct-probe|fixture direct terminal, cols#138,\n"
        ))
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        lookup_environment = dict(self.environment)
        for name in ("TERMINFO", "TERMINFO_DIRS", "PREFIX"):
            lookup_environment.pop(name, None)
        for name, columns in (("alacritty-installer-probe", 137), ("alacritty-installer-direct-probe", 138)):
            lookup = subprocess.run(["infocmp", name], env=lookup_environment,
                                    capture_output=True, text=True)
            self.assertEqual(lookup.returncode, 0, lookup.stderr)
            self.assertIn(f"cols#{columns}", lookup.stdout)
            self.assertIn(str(user_terminfo), lookup.stdout)
        for path, content in before.items():
            self.assertEqual(path.read_bytes(), content)

    def test_gnome_new_window_action_keeps_one_action_and_translated_label(self):
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        launcher = self.prefix / "share/applications/Termust.desktop"
        config = configparser.ConfigParser(interpolation=None)
        config.optionxform = str
        config.read(launcher)
        self.assertEqual(config["Desktop Entry"]["Actions"], "new-window;")
        self.assertFalse(config.has_section("Desktop Action New"))
        action = config["Desktop Action new-window"]
        self.assertEqual(action["Name"], "New Window")
        self.assertEqual(action["Name[pt_BR]"], "Nova janela")
        self.assertEqual(action["Exec"], config["Desktop Entry"]["Exec"])
        if shutil.which("desktop-file-validate"):
            subprocess.run(["desktop-file-validate", str(launcher)], check=True)

    @unittest.skipUnless(shutil.which("tic"), "tic unavailable")
    def test_unwritable_user_terminfo_warns_without_losing_binary_install(self):
        (Path(self.environment["HOME"]) / ".terminfo").write_text("keep this file")
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((self.prefix / "bin/termust").read_bytes(), BINARY)
        self.assertIn("Warning: terminfo could not be updated in", result.stderr)
        self.assertEqual((Path(self.environment["HOME"]) / ".terminfo").read_text(), "keep this file")

    def test_managed_workspace_migrates_with_backup_and_reinstalls_safely(self):
        binary, launcher, receipt = self.managed_alacritty()
        original = {path: path.read_bytes() for path in (binary, launcher, receipt)}
        settings = Path(self.environment["HOME"]) / ".config/alacritty/alacritty.toml"
        sessions = Path(self.environment["HOME"]) / ".local/state/alacritty/tabs.json"
        for path, data in ((settings, b"[font]\nsize=14.2\n"), (sessions, b'{"tabs":[]}\n')):
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
            original[path] = data
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(settings.read_bytes(), original[settings])
        self.assertEqual(sessions.read_bytes(), original[sessions])
        self.assertIn("Migrated the previous Alacritty Workspace", result.stdout)
        self.assertTrue(binary.is_symlink())
        self.assertEqual(os.readlink(binary), "termust")
        self.assertEqual(binary.resolve(), self.prefix / "bin/termust")
        self.assertFalse(launcher.exists())
        self.assertEqual(receipt.read_bytes(), original[receipt])
        self.assertEqual(subprocess.check_output([str(binary), "--version"]), b"termust 0.18.0 (fixture)\n")
        record = self.prefix / "share/doc/termust/legacy-alacritty-migration.json"
        marker = json.loads(record.read_text())
        backup = self.prefix / marker["backup_directory"]
        for path in (binary, launcher, receipt):
            self.assertEqual((backup / path.name).read_bytes(), original[path])
        self.assertTrue(os.access(backup / "alacritty", os.X_OK))
        self.assertEqual(marker["binary_sha256"], hashlib.sha256(original[binary]).hexdigest())
        self.assertEqual(marker["launcher_sha256"], hashlib.sha256(original[launcher]).hexdigest())
        self.assertEqual(subprocess.check_output([str(backup / "alacritty"), "--version"]),
            b"alacritty 0.18.0-dev (c14a29f6)\n")
        repeated = self.run_installer()
        self.assertEqual(repeated.returncode, 0, repeated.stdout + repeated.stderr)
        self.assertTrue(binary.is_symlink())
        self.assertEqual(json.loads(record.read_text()), marker)
        self.assertEqual(list(binary.parent.glob(".termust-*")), [])

    def test_managed_migration_preserves_a_running_previous_terminal(self):
        binary, launcher, receipt = self.managed_alacritty()
        process = subprocess.Popen([str(binary)])
        try:
            result = self.run_installer()
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue(binary.is_symlink())
            self.assertIsNone(process.poll())
        finally:
            process.terminate()
            process.wait(timeout=5)

    def test_changed_receipt_binary_or_launcher_is_not_migrated(self):
        cases = ["receipt", "binary", "name", "command", "new_action", "working_directory", "symlink", "hardlink"]
        for case in cases:
            with self.subTest(case=case):
                shutil.rmtree(self.prefix, ignore_errors=True)
                binary, launcher, receipt = self.managed_alacritty()
                if case == "receipt":
                    receipt.write_text("other terminal\n")
                elif case == "binary":
                    binary.write_bytes(b'#!/usr/bin/env bash\nprintf "alacritty 0.19.0-dev (12345678)\\n"\n')
                elif case == "name":
                    launcher.write_text(launcher.read_text().replace("Name=Alacritty Workspace", "Name=My terminal"))
                elif case == "command":
                    launcher.write_text(launcher.read_text().replace('Exec="', 'Exec="--custom" "', 1))
                elif case == "new_action":
                    launcher.write_text(launcher.read_text().replace("Name=New Terminal", "Name=New Terminal\nExec=custom"))
                elif case == "working_directory":
                    launcher.write_text(launcher.read_text().replace("Type=Application", "Type=Application\nPath=/tmp"))
                elif case == "symlink":
                    target = self.directory / "unmanaged-alacritty"
                    target.write_bytes(binary.read_bytes())
                    binary.unlink()
                    binary.symlink_to(target)
                elif case == "hardlink":
                    os.link(binary, self.directory / "retained-alacritty-hardlink")
                original = {path: path.read_bytes() for path in (binary, launcher, receipt)}
                result = self.run_installer()
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                for path, content in original.items():
                    self.assertEqual(path.read_bytes(), content)
                self.assertFalse((self.prefix / "share/doc/termust/legacy-alacritty-migration.json").exists())

    def test_managed_migration_handles_desktop_reserved_path_characters(self):
        self.prefix = self.directory / 'old terminal $money `name` "quote" \\slash %test'
        binary, launcher, receipt = self.managed_alacritty()
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue(binary.is_symlink())
        self.assertFalse(launcher.exists())
        self.assertEqual(subprocess.check_output([str(binary), "--version"]), b"termust 0.18.0 (fixture)\n")

    def test_legacy_backup_collision_preserves_previous_install(self):
        binary, launcher, receipt = self.managed_alacritty()
        original = {path: path.read_bytes() for path in (binary, launcher, receipt)}
        backup = self.prefix / "share/doc/termust/legacy-alacritty" / receipt.read_text().strip()
        backup.mkdir(parents=True)
        (backup / "alacritty").write_text("keep this backup\n")
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("Warning: previous fork integration could not be migrated", result.stderr)
        self.assertEqual((self.prefix / "bin/termust").read_bytes(), BINARY)
        self.assertEqual((backup / "alacritty").read_text(), "keep this backup\n")
        for path, content in original.items():
            self.assertEqual(path.read_bytes(), content)

    def test_no_desktop_preserves_previous_managed_integration(self):
        binary, launcher, receipt = self.managed_alacritty()
        original = {path: path.read_bytes() for path in (binary, launcher, receipt)}
        result = self.run_installer("--no-desktop")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for path, content in original.items():
            self.assertEqual(path.read_bytes(), content)
        self.assertFalse((self.prefix / "share/applications/Termust.desktop").exists())

    def test_alacritty_executable_launcher_settings_and_sessions_coexist(self):
        retained = {
            self.prefix / "bin/alacritty": b"existing Alacritty executable\n",
            self.prefix / "share/applications/Alacritty.desktop": b"Name=Alacritty Workspace\n",
            self.prefix / "share/icons/hicolor/scalable/apps/Alacritty.svg": b"existing icon\n",
            Path(self.environment["HOME"]) / ".config/alacritty/alacritty.toml": b"[window]\nopacity=0.9\n",
            Path(self.environment["HOME"]) / ".local/state/alacritty/tabs.json": b'{"tabs":[]}\n',
        }
        for path, content in retained.items():
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((self.prefix / "bin/termust").read_bytes(), BINARY)
        self.assertIn("Name=Termust\n", (self.prefix / "share/applications/Termust.desktop").read_text())
        for path, content in retained.items():
            self.assertEqual(path.read_bytes(), content)

    def test_pre_termust_release_points_to_original_installer(self):
        binary = self.old_install()
        arguments = self.fake_remote()
        version = "0.18.0-workspace.c14a29f6.66afacc21311"
        result = self.run_installer(*arguments, "--version", version, remote=True)
        self.assert_preserved(result, binary)
        self.assertIn(f"releases/{version}/install.sh", result.stderr)
        self.assertFalse((self.directory / "requests").exists())

    def test_wrong_product_binary_preserves_installed_terminal(self):
        binary = self.old_install()
        self.make_archive(binary=b"#!/usr/bin/env bash\nprintf 'alacritty 0.18.0\\n'\n")
        result = self.run_installer()
        self.assert_preserved(result, binary)
        self.assertIn("not Termust", result.stderr)

    def test_checksum_mismatch_preserves_installed_terminal(self):
        binary = self.old_install()
        Path(str(self.archive) + ".sha256").write_text("0" * 64 + "\n")
        result = self.run_installer()
        self.assert_preserved(result, binary)
        self.assertIn("checksum mismatch", result.stderr)

    def test_bad_checksum_file_preserves_installed_terminal(self):
        binary = self.old_install()
        Path(str(self.archive) + ".sha256").write_text("not a checksum\n")
        result = self.run_installer()
        self.assert_preserved(result, binary)
        self.assertIn("invalid SHA256", result.stderr)

    def test_explicit_checksum_file(self):
        checksum = self.directory / "checksum.txt"
        Path(str(self.archive) + ".sha256").rename(checksum)
        result = self.run_installer("--checksum-file", str(checksum))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_unsafe_archive_entries_preserve_installed_terminal(self):
        for name, kind in (
            ("../outside", tarfile.REGTYPE),
            ("/tmp/alacritty-installer-escape", tarfile.REGTYPE),
            ("termust/link", tarfile.SYMTYPE),
            ("termust/hardlink", tarfile.LNKTYPE),
            ("termust/device", tarfile.FIFOTYPE),
            ("different-root/file", tarfile.REGTYPE),
            ("termust/bin/termust", tarfile.REGTYPE),
        ):
            with self.subTest(name=name):
                binary = self.old_install()
                member = tarfile.TarInfo(name)
                member.type = kind
                member.linkname = "../../outside"
                data = b"unsafe" if kind == tarfile.REGTYPE else None
                member.size = len(data) if data is not None else 0
                self.make_archive(extra=[(member, data)])
                result = self.run_installer()
                self.assert_preserved(result, binary)
                self.assertIn("unsafe release archive", result.stderr)
        self.assertFalse((self.directory / "outside").exists())

    def test_incompatible_binary_preserves_installed_terminal(self):
        binary = self.old_install()
        self.make_archive(binary=b"#!/usr/bin/env bash\nprintf 'missing runtime' >&2\nexit 127\n")
        result = self.run_installer()
        self.assert_preserved(result, binary)
        self.assertIn("cannot run on this machine", result.stderr)
        self.assertIn("missing runtime", result.stderr)

    def test_prefix_with_spaces_and_desktop_reserved_characters(self):
        self.prefix = self.directory / 'my terminal $money `name` "quote" \\slash %test'
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        launcher = (self.prefix / "share/applications/Termust.desktop").read_text()
        self.assertIn("Name=Termust\n", launcher)
        command = next(line[5:] for line in launcher.splitlines() if line.startswith("Exec="))
        first_unescape = command.replace("\\\\", "\\")
        self.assertTrue(first_unescape.startswith('"') and first_unescape.endswith('"'))
        value = first_unescape[1:-1]
        for character in ('"', "`", "$", "\\"):
            value = value.replace("\\" + character, character)
        self.assertEqual(value.replace("%%", "%"), str(self.prefix / "bin/termust"))
        self.assertEqual(launcher.count("Exec="), 3)

    def test_no_desktop(self):
        result = self.run_installer("--no-desktop")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue((self.prefix / "bin/termust").exists())
        self.assertFalse((self.prefix / "share/applications").exists())
        self.assertFalse((self.prefix / "share/icons").exists())

    def test_remote_latest_from_pipe(self):
        arguments = self.fake_remote()
        result = self.run_installer(*arguments, remote=True, source=INSTALLER.read_text())
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        requests = (self.directory / "requests").read_text().splitlines()
        self.assertEqual(requests, [
            "latest-linux-x86_64.txt",
            f"releases/{VERSION}/termust-linux-x86_64.tar.gz",
            f"releases/{VERSION}/termust-linux-x86_64.tar.gz.sha256",
        ])
        self.assertEqual((self.prefix / "bin/termust").read_bytes(), BINARY)

    def test_remote_pinned_release_skips_latest_pointer(self):
        arguments = self.fake_remote()
        result = self.run_installer(*arguments, "--version", VERSION, remote=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("latest", (self.directory / "requests").read_text())

    def test_invalid_latest_pointer_preserves_installed_terminal(self):
        binary = self.old_install()
        arguments = self.fake_remote(pointer="../../escape")
        result = self.run_installer(*arguments, remote=True)
        self.assert_preserved(result, binary)
        self.assertEqual((self.directory / "requests").read_text(), "latest-linux-x86_64.txt\n")

    def test_pinned_package_version_mismatch_preserves_installed_terminal(self):
        binary = self.old_install()
        result = self.run_installer("--version", "other-version")
        self.assert_preserved(result, binary)
        self.assertIn("VERSION does not match", result.stderr)

    def test_non_https_base_url_preserves_installed_terminal(self):
        binary = self.old_install()
        result = self.run_installer("--base-url", "http://example.test", remote=True)
        self.assert_preserved(result, binary)
        self.assertIn("HTTPS", result.stderr)

    def test_truncated_pipe_does_not_execute_installer(self):
        binary = self.old_install()
        source = INSTALLER.read_text().rsplit('main "$@"', 1)[0]
        result = self.run_installer(source=source)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(binary.read_bytes(), b"previous terminal\n")
        self.assertFalse((self.prefix / "share").exists())

    def test_replaces_executable_while_previous_terminal_is_running(self):
        binary = self.prefix / "bin/termust"
        binary.parent.mkdir(parents=True)
        shutil.copy2(shutil.which("sleep"), binary)
        process = subprocess.Popen([str(binary), "30"])
        try:
            result = self.run_installer()
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(binary.read_bytes(), BINARY)
            self.assertIsNone(process.poll())
        finally:
            process.terminate()
            process.wait(timeout=5)

    def test_optional_integration_failure_keeps_successful_binary_install(self):
        self.prefix.mkdir()
        (self.prefix / "share").write_text("blocked integration directory")
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((self.prefix / "bin/termust").read_bytes(), BINARY)
        self.assertIn("Warning: some optional", result.stderr)

    def test_destination_directory_is_not_used_as_an_executable(self):
        binary = self.prefix / "bin/termust"
        binary.mkdir(parents=True)
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(list(binary.iterdir()), [])
        self.assertEqual(list(binary.parent.glob(".termust-install.*")), [])

    def fixture_release(self, app_version="0.18.0", build_timestamp=1000, metadata_changes=None):
        binary_version = f"termust {app_version} (fixture-{build_timestamp})"
        binary = f"#!/usr/bin/env bash\nprintf '{binary_version}\\n'\n".encode()
        version = f"{app_version}-termust.fixture.{build_timestamp}"
        metadata = {
            "version": version,
            "platform": "linux-x86_64",
            "package_version": app_version,
            "binary_version": binary_version,
            "binary_sha256": hashlib.sha256(binary).hexdigest(),
            "minimum_glibc": "2.31",
            "build_timestamp": build_timestamp,
        }
        if metadata_changes:
            metadata.update(metadata_changes)
        self.make_archive(binary=binary, version=version, metadata=metadata)
        return binary, version

    def assert_install_success(self, *arguments):
        result = self.run_installer(*arguments)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def test_release_metadata_hash_mismatch_preserves_installed_terminal(self):
        binary = self.old_install()
        self.fixture_release(metadata_changes={"binary_sha256": "0" * 64})
        result = self.run_installer()
        self.assert_preserved(result, binary)
        self.assertRegex(result.stderr.lower(), "hash|checksum")

    def test_release_metadata_version_mismatch_preserves_installed_terminal(self):
        binary = self.old_install()
        self.fixture_release(metadata_changes={"version": "other-version"})
        result = self.run_installer()
        self.assert_preserved(result, binary)
        self.assertIn("version", result.stderr.lower())

    def test_required_glibc_is_checked_before_replacing_terminal(self):
        binary = self.old_install()
        self.fixture_release(metadata_changes={"minimum_glibc": "999.0"})
        result = self.run_installer()
        self.assert_preserved(result, binary)
        self.assertIn("glibc", result.stderr.lower())
        self.assertIn("999.0", result.stderr)

    def test_older_app_version_requires_explicit_downgrade(self):
        newer, _ = self.fixture_release(app_version="0.19.0", build_timestamp=1000)
        self.assert_install_success()
        older, _ = self.fixture_release(app_version="0.18.0", build_timestamp=2000)
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("--allow-downgrade", result.stderr)
        self.assertEqual((self.prefix / "bin/termust").read_bytes(), newer)
        self.assert_install_success("--allow-downgrade")
        self.assertEqual((self.prefix / "bin/termust").read_bytes(), older)

    def test_older_build_of_same_app_version_requires_explicit_downgrade(self):
        newer, _ = self.fixture_release(build_timestamp=2000)
        self.assert_install_success()
        self.fixture_release(build_timestamp=1000)
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("--allow-downgrade", result.stderr)
        self.assertEqual((self.prefix / "bin/termust").read_bytes(), newer)

    def test_legacy_binary_version_blocks_an_older_release_without_receipt(self):
        binary = self.prefix / "bin/termust"
        binary.parent.mkdir(parents=True)
        previous = b"#!/usr/bin/env bash\nprintf 'termust 0.20.0 (legacy)\\n'\n"
        binary.write_bytes(previous)
        binary.chmod(0o755)
        self.fixture_release(app_version="0.18.0")
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("--allow-downgrade", result.stderr)
        self.assertEqual(binary.read_bytes(), previous)

    def test_rollback_restores_previous_binary_launcher_receipt_and_preserves_settings(self):
        previous, previous_version = self.fixture_release(build_timestamp=1000)
        self.assert_install_success()
        launcher = self.prefix / "share/applications/Termust.desktop"
        launcher.write_text(launcher.read_text().replace("Name=Termust", "Name=My Termust", 1))
        previous_launcher = launcher.read_bytes()
        receipt = self.prefix / "share/doc/termust/install.json"
        previous_receipt = receipt.read_bytes()
        settings = Path(self.environment["HOME"]) / ".config/termust/termust.toml"
        settings.parent.mkdir(parents=True)
        settings.write_text("[window]\nopacity=0.8\n")
        self.fixture_release(build_timestamp=2000)
        self.assert_install_success()
        self.archive.unlink()
        Path(str(self.archive) + ".sha256").unlink()
        result = self.assert_install_success("--rollback")
        self.assertIn(previous_version, result.stdout)
        self.assertEqual((self.prefix / "bin/termust").read_bytes(), previous)
        self.assertEqual(launcher.read_bytes(), previous_launcher)
        self.assertEqual(receipt.read_bytes(), previous_receipt)
        self.assertEqual(settings.read_text(), "[window]\nopacity=0.8\n")

    def test_reinstall_repairs_launcher_without_discarding_previous_rollback(self):
        previous, _ = self.fixture_release(build_timestamp=1000)
        self.assert_install_success()
        self.fixture_release(build_timestamp=2000)
        self.assert_install_success()
        backup = self.prefix / "share/termust/rollback"
        before = {str(path.relative_to(backup)): path.read_bytes() for path in backup.rglob("*") if path.is_file()}
        self.assertIn("manifest.json", before)
        launcher = self.prefix / "share/applications/Termust.desktop"
        launcher.write_text("broken launcher\n")
        self.assert_install_success()
        self.assertIn("Name=Termust\n", launcher.read_text())
        after = {str(path.relative_to(backup)): path.read_bytes() for path in backup.rglob("*") if path.is_file()}
        self.assertEqual(after, before)
        self.assert_install_success("--rollback")
        self.assertEqual((self.prefix / "bin/termust").read_bytes(), previous)

    def test_corrupt_rollback_payload_preserves_current_terminal(self):
        previous, _ = self.fixture_release(build_timestamp=1000)
        self.assert_install_success()
        current, _ = self.fixture_release(build_timestamp=2000)
        self.assert_install_success()
        backup = self.prefix / "share/termust/rollback"
        payloads = [path for path in backup.rglob("*") if path.is_file() and path.read_bytes() == previous]
        self.assertEqual(len(payloads), 1)
        payloads[0].write_bytes(b"corrupt backup\n")
        result = self.run_installer("--rollback")
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((self.prefix / "bin/termust").read_bytes(), current)
        self.assertRegex(result.stderr.lower(), "hash|checksum|corrupt|backup")

    def test_rollback_without_snapshot_preserves_current_terminal(self):
        current, _ = self.fixture_release()
        self.assert_install_success()
        result = self.run_installer("--rollback")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((self.prefix / "bin/termust").read_bytes(), current)

    def test_rollback_rejects_archive_and_version_arguments(self):
        for arguments in (("--archive", str(self.archive)), ("--version", VERSION)):
            with self.subTest(arguments=arguments):
                result = self.run_installer("--rollback", *arguments)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(self.prefix.exists())

    def test_check_is_read_only_and_verifies_existing_installation(self):
        self.environment["PATH"] = str(self.prefix / "bin") + os.pathsep + self.environment["PATH"]
        self.environment["XDG_DATA_DIRS"] = str(self.prefix / "share") + ":/usr/share"
        self.fixture_release()
        self.assert_install_success()
        before = {str(path.relative_to(self.prefix)): (path.read_bytes(), path.stat().st_mtime_ns)
                  for path in self.prefix.rglob("*") if path.is_file()}
        result = self.run_installer("--check")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        after = {str(path.relative_to(self.prefix)): (path.read_bytes(), path.stat().st_mtime_ns)
                 for path in self.prefix.rglob("*") if path.is_file()}
        self.assertEqual(after, before)
        self.assertIn(str(self.prefix / "bin/termust"), result.stdout)

    def test_check_detects_modified_installed_binary_without_repairing_it(self):
        self.environment["PATH"] = str(self.prefix / "bin") + os.pathsep + self.environment["PATH"]
        self.environment["XDG_DATA_DIRS"] = str(self.prefix / "share") + ":/usr/share"
        self.fixture_release()
        self.assert_install_success()
        binary = self.prefix / "bin/termust"
        modified = binary.read_bytes() + b"\n"
        binary.write_bytes(modified)
        result = self.run_installer("--check")
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertRegex(result.stderr.lower(), "hash|checksum|match|modified")
        self.assertEqual(binary.read_bytes(), modified)

    def test_check_missing_installation_does_not_create_prefix(self):
        result = self.run_installer("--check")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.prefix.exists())

    def test_path_shadowing_is_reported_without_replacing_other_binary(self):
        commands = self.directory / "commands"
        commands.mkdir()
        other = commands / "termust"
        other.write_text("#!/usr/bin/env bash\nprintf 'termust 0.17.0 (other)\\n'\n")
        other.chmod(0o755)
        original = other.read_bytes()
        self.environment["PATH"] = str(commands) + os.pathsep + str(self.prefix / "bin") + os.pathsep + self.environment["PATH"]
        self.fixture_release()
        result = self.assert_install_success()
        output = result.stdout + result.stderr
        self.assertIn(str(other), output)
        self.assertIn("PATH", output)
        self.assertIn("warning", output.lower())
        self.assertEqual(other.read_bytes(), original)

    def test_nonstandard_prefix_reports_desktop_discovery_requirement(self):
        self.environment["XDG_DATA_HOME"] = str(self.directory / "xdg-user")
        self.environment["XDG_DATA_DIRS"] = "/usr/local/share:/usr/share"
        self.fixture_release()
        result = self.assert_install_success()
        output = result.stdout + result.stderr
        self.assertIn("XDG_DATA_DIRS", output)
        self.assertIn(str(self.prefix / "share"), output)

    @unittest.skipIf(os.geteuid() == 0, "filesystem permissions require an unprivileged user")
    def test_receipt_write_failure_preserves_previous_installation(self):
        self.fixture_release(build_timestamp=1000)
        self.assert_install_success()
        previous, _ = self.fixture_release(build_timestamp=2000)
        self.assert_install_success()
        backup = self.prefix / "share/termust/rollback"
        previous_snapshot = {str(path.relative_to(backup)): path.read_bytes()
                             for path in backup.rglob("*") if path.is_file()}
        receipt = self.prefix / "share/doc/termust/install.json"
        previous_receipt = receipt.read_bytes()
        previous_version = (receipt.parent / "VERSION").read_bytes()
        launcher = self.prefix / "share/applications/Termust.desktop"
        previous_launcher = launcher.read_bytes()
        receipt.parent.chmod(0o555)
        self.fixture_release(build_timestamp=3000)
        try:
            result = self.run_installer()
        finally:
            receipt.parent.chmod(0o755)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((self.prefix / "bin/termust").read_bytes(), previous)
        self.assertEqual(receipt.read_bytes(), previous_receipt)
        self.assertEqual((receipt.parent / "VERSION").read_bytes(), previous_version)
        self.assertEqual(launcher.read_bytes(), previous_launcher)
        snapshot_after = {str(path.relative_to(backup)): path.read_bytes()
                          for path in backup.rglob("*") if path.is_file()}
        self.assertEqual(snapshot_after, previous_snapshot)
        self.assert_install_success()

    def test_rollback_preserves_custom_file_permissions(self):
        previous, _ = self.fixture_release(build_timestamp=1000)
        self.assert_install_success()
        binary = self.prefix / "bin/termust"
        launcher = self.prefix / "share/applications/Termust.desktop"
        binary.chmod(0o750)
        launcher.chmod(0o600)
        self.fixture_release(build_timestamp=2000)
        self.assert_install_success()
        self.assert_install_success("--rollback")
        self.assertEqual(binary.read_bytes(), previous)
        self.assertEqual(binary.stat().st_mode & 0o777, 0o750)
        self.assertEqual(launcher.stat().st_mode & 0o777, 0o600)

    def test_check_rejects_a_higher_priority_desktop_launcher(self):
        self.environment["PATH"] = str(self.prefix / "bin") + os.pathsep + self.environment["PATH"]
        self.environment["XDG_DATA_DIRS"] = str(self.prefix / "share") + ":/usr/share"
        self.fixture_release()
        self.assert_install_success()
        other = self.directory / "xdg-user/applications/Termust.desktop"
        other.parent.mkdir(parents=True)
        other.write_text("[Desktop Entry]\nExec=/other/termust\n")
        self.environment["XDG_DATA_HOME"] = str(other.parent.parent)
        result = self.run_installer("--check")
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(str(other), result.stderr)
        self.assertEqual(other.read_text(), "[Desktop Entry]\nExec=/other/termust\n")

    def test_failed_new_desktop_integration_preserves_managed_legacy_launcher(self):
        binary, launcher, receipt = self.managed_alacritty()
        before = {path: path.read_bytes() for path in (binary, launcher, receipt)}
        external = self.directory / "shared-applications"
        launcher.parent.rename(external)
        launcher.parent.symlink_to(external, target_is_directory=True)
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("Warning", result.stderr)
        self.assertFalse(binary.is_symlink())
        for path, data in before.items():
            self.assertEqual(path.read_bytes(), data)
        self.assertFalse((external / "Termust.desktop").exists())

    def test_metadata_free_package_cannot_replace_a_dated_release_without_override(self):
        current, _ = self.fixture_release(build_timestamp=2000)
        self.assert_install_success()
        self.make_archive()
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("--allow-downgrade", result.stderr)
        self.assertEqual((self.prefix / "bin/termust").read_bytes(), current)
        self.assert_install_success("--allow-downgrade")
        self.assertEqual((self.prefix / "bin/termust").read_bytes(), BINARY)

    def test_concurrent_installer_preserves_binary_and_releases_lock(self):
        binary = self.old_install()
        with (self.prefix / ".termust-install.lock").open("w") as lock:
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
            result = self.run_installer()
            self.assert_preserved(result, binary)
            self.assertIn("another Termust installation", result.stderr)
        self.assert_install_success()
        self.assertEqual(binary.read_bytes(), BINARY)

    def test_help_does_not_install(self):
        result = self.run_installer("--help")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("python3", result.stdout)
        self.assertFalse(self.prefix.exists())


if __name__ == "__main__":
    unittest.main(verbosity=2)
