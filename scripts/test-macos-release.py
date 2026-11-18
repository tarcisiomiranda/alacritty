import hashlib
import importlib.util
import io
import json
import plistlib
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]


def load_script(name):
    path = ROOT / "install.sh" if name == "install.sh" else ROOT / "scripts" / name
    code = path.read_text().split("<<'TERMUST_PYTHON'\n", 1)[1].split("\ntry:\n", 1)[0]
    namespace = {}
    exec(compile(code, name, "exec"), namespace)
    return namespace


BUILDER = load_script("build-macos-release.sh")
INSTALLER = load_script("install.sh")


def create_archive(path, entries):
    with tarfile.open(path, "w:gz") as tar:
        for name, data in entries.items():
            item = tarfile.TarInfo(name)
            item.mode = 0o755 if name.endswith("/termust") else 0o644
            item.size = len(data)
            tar.addfile(item, io.BytesIO(data))


class ReleaseTests(unittest.TestCase):
    def test_source_package_contains_the_embedded_personal_icons(self):
        spec = importlib.util.spec_from_file_location("macos_source", ROOT / "scripts/package-macos-source.py")
        package = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(package)
        package.DIST = self.root / "source-package"
        package.main()
        with tarfile.open(package.DIST / "source.tar.gz") as archive:
            for name in ("cat", "dog", "fox", "robot"):
                entry = "termust-source/extra/icons/" + name + ".png"
                self.assertEqual(archive.extractfile(entry).read(), (ROOT / "extra/icons" / (name + ".png")).read_bytes())
            self.assertIsNotNone(archive.getmember("termust-source/extra/logo/termust.png"))

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source_archive = self.root / "source.tar.gz"
        template = (ROOT / "install.sh").read_bytes()
        create_archive(self.source_archive, {
            "termust-source/extra/osx/Termust.app/Contents/Info.plist": b"plist",
            "termust-source/extra/osx/Termust.app/Contents/Resources/icon": b"icon",
            "termust-source/LICENSE-APACHE": b"license", "termust-source/LICENSE-MIT": b"license",
            "termust-source/Cargo.lock": b"lock",
            "termust-source/install.sh": template,
        })
        self.source_hash = hashlib.sha256(self.source_archive.read_bytes()).hexdigest()
        self.objects = {}
        self.calls = []
        self.corrupt_download = False
        self.installer_env = None
        self.build_dir = None
        patches = [
            mock.patch.object(BUILDER["platform"], "system", return_value="Darwin"),
            mock.patch.object(BUILDER["platform"], "machine", return_value="arm64"),
            mock.patch.object(BUILDER["shutil"], "which", return_value="/fixture/tool"),
            mock.patch.object(BUILDER["subprocess"], "run", side_effect=self.run_command),
            mock.patch.object(BUILDER["sys"], "argv", ["build"]),
            mock.patch.dict(BUILDER, {"SOURCE_SHA256": self.source_hash}),
        ]
        for patch in patches:
            patch.start()
            self.addCleanup(patch.stop)

    def run_command(self, command, **kwargs):
        self.calls.append(command)
        if "rustc" in command:
            return mock.Mock(returncode=0, stdout="host: aarch64-apple-darwin\n", stderr="")
        if command[0] == "curl":
            name = command[-1].split("/file/termust/", 1)[1]
            data = self.source_archive.read_bytes() if name.startswith("macos/source/") else self.objects[name]
            if self.corrupt_download and name.startswith("macos/binary/"):
                data = b"corrupted"
            Path(command[command.index("--output") + 1]).write_bytes(data)
        elif "cargo" in command:
            self.build_dir = kwargs["cwd"]
            executable = Path(kwargs["env"]["CARGO_TARGET_DIR"]) / "aarch64-apple-darwin/release/termust"
            executable.parent.mkdir(parents=True)
            executable.write_bytes(b"fixture binary")
            executable.chmod(0o755)
        elif command[0] == "b2":
            self.objects[command[-1]] = Path(command[-2]).read_bytes()
        elif command[0] == "bash":
            self.installer_env = kwargs["env"]
            binary = Path(kwargs["env"]["HOME"]) / "Applications/Termust.app/Contents/MacOS/termust"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"fixture binary")
        return mock.Mock(returncode=0)

    def test_build_and_publication_order_and_content(self):
        BUILDER["main"]()
        uploads = [command[-1] for command in self.calls if command[0] == "b2"]
        self.assertEqual(len(uploads), 2)
        self.assertTrue(uploads[0].startswith("macos/binary/"))
        self.assertEqual(uploads[1], "latest-macos-arm64.json")
        self.assertTrue(any(command[0] == "bash" for command in self.calls))
        self.assertEqual(self.installer_env["PATH"], "/usr/bin:/bin:/usr/sbin:/sbin")
        digest = hashlib.sha256(self.objects[uploads[0]]).hexdigest()
        manifest = json.loads(self.objects["latest-macos-arm64.json"])
        self.assertEqual(manifest["sha256"], digest)
        self.assertEqual(manifest["archive"], uploads[0])
        self.assertFalse(any(name.endswith("install.sh") for name in uploads))
        self.assertTrue(any(command[:4] == ["mise", "exec", "rust@1.89.0", "--"] and "cargo" in command for command in self.calls))
        with tarfile.open(fileobj=io.BytesIO(self.objects[uploads[0]]), mode="r:gz") as tar:
            self.assertIn("Termust.app/Contents/MacOS/termust", tar.getnames())

    def test_bad_public_archive_does_not_publish_installer(self):
        self.corrupt_download = True
        with self.assertRaisesRegex(RuntimeError, "checksum mismatch"):
            BUILDER["main"]()
        self.assertNotIn("latest-macos-arm64.json", self.objects)

    def test_bad_source_does_not_upload(self):
        BUILDER["SOURCE_SHA256"] = "0" * 64
        with self.assertRaisesRegex(RuntimeError, "source archive checksum mismatch"):
            BUILDER["main"]()
        self.assertEqual(self.objects, {})

    def test_explicit_source_directory_builds_local_changes(self):
        with tarfile.open(self.source_archive, "r:gz") as archive:
            archive.extractall(self.root)
        source = self.root / "termust-source"
        BUILDER["sys"].argv = ["build", "--source-dir", str(source)]
        BUILDER["main"]()
        self.assertEqual(self.build_dir, source.resolve())
        self.assertFalse(any(command[0] == "curl" and "/macos/source/" in command[-1] for command in self.calls))


class BinaryInstallerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.home = Path(self.temporary.name)
        self.archive = self.home / "binary.tar.gz"
        self.make_archive()
        self.manifest = {"schema": 1, "platform": "macos-arm64", "sha256": hashlib.sha256(self.archive.read_bytes()).hexdigest()}
        self.manifest["archive"] = f"macos/binary/{self.manifest['sha256']}.tar.gz"
        self.calls = []
        patches = [
            mock.patch.object(INSTALLER["platform"], "system", return_value="Darwin"),
            mock.patch.object(INSTALLER["platform"], "machine", return_value="arm64"),
            mock.patch.object(INSTALLER["Path"], "home", return_value=self.home),
            mock.patch.object(INSTALLER["shutil"], "which", return_value="/fixture/tool"),
            mock.patch.object(INSTALLER["subprocess"], "run", side_effect=self.run_command),
            mock.patch.object(INSTALLER["sys"], "argv", ["install"]),
        ]
        for patch in patches:
            patch.start()
            self.addCleanup(patch.stop)

    def make_archive(self, extra=None):
        info = plistlib.dumps({"CFBundleIdentifier": "com.tarcisiomiranda.termust"})
        entries = {"Termust.app/Contents/Info.plist": info, "Termust.app/Contents/MacOS/termust": b"binary"}
        if extra:
            entries[extra] = b"unsafe"
        create_archive(self.archive, entries)

    def run_command(self, command, **kwargs):
        self.calls.append(command)
        if command[0] == "curl":
            data = json.dumps(self.manifest).encode() if command[-1].endswith(".json") else self.archive.read_bytes()
            Path(command[command.index("--output") + 1]).write_bytes(data)
        return mock.Mock(returncode=0, stdout="termust 0.18.0-dev\n", stderr="")

    def test_installs_verified_binary_without_rust(self):
        INSTALLER["main"]()
        self.assertTrue((self.home / "Applications/Termust.app/Contents/MacOS/termust").is_file())
        self.assertTrue(any(command[:2] == ["codesign", "--verify"] for command in self.calls))
        self.assertFalse(any("cargo" in command or "rustc" in command for command in self.calls))

    def test_checksum_and_existing_app_are_preserved(self):
        self.manifest["sha256"] = "0" * 64
        self.manifest["archive"] = f"macos/binary/{self.manifest['sha256']}.tar.gz"
        with self.assertRaisesRegex(RuntimeError, "checksum mismatch"):
            INSTALLER["main"]()
        app = self.home / "Applications/Termust.app"
        app.mkdir()
        (app / "keep").write_text("keep")
        with self.assertRaisesRegex(RuntimeError, "unrecognized or incomplete application"):
            INSTALLER["main"]()
        self.assertEqual((app / "keep").read_text(), "keep")

    def existing_app(self, managed=False):
        directory = self.home / "Applications"
        app = directory / ".termust/releases/legacy/Termust.app" if managed else directory / "Termust.app"
        (app / "Contents/MacOS").mkdir(parents=True)
        (app / "Contents/Info.plist").write_bytes(plistlib.dumps({"CFBundleIdentifier": "com.tarcisiomiranda.termust"}))
        binary = app / "Contents/MacOS/termust"
        binary.write_bytes(b"previous binary")
        binary.chmod(0o755)
        if managed:
            (directory / "Termust.app").symlink_to(".termust/releases/legacy/Termust.app")
        return app

    def test_updates_managed_link_and_reinstall_preserves_previous_version(self):
        previous = self.existing_app(managed=True)
        INSTALLER["main"]()
        destination = self.home / "Applications/Termust.app"
        current = destination.resolve()
        backup = destination.parent / ".termust/previous.app"
        self.assertEqual((current / "Contents/MacOS/termust").read_bytes(), b"binary")
        self.assertEqual(backup.resolve(), previous.resolve())
        INSTALLER["main"]()
        self.assertEqual(destination.resolve(), current)
        self.assertEqual(backup.resolve(), previous.resolve())
        self.assertEqual((previous / "Contents/MacOS/termust").read_bytes(), b"previous binary")

    def test_migrates_real_app_without_losing_previous_bundle_or_settings(self):
        self.existing_app()
        settings = self.home / "settings-to-preserve"
        settings.write_text("user settings")
        INSTALLER["main"]()
        destination = self.home / "Applications/Termust.app"
        self.assertTrue(destination.is_symlink())
        backup = destination.parent / ".termust/previous.app"
        self.assertEqual((backup / "Contents/MacOS/termust").read_bytes(), b"previous binary")
        self.assertEqual(settings.read_text(), "user settings")

    def test_bad_update_checksum_preserves_installed_application(self):
        app = self.existing_app(managed=True)
        self.manifest["sha256"] = "0" * 64
        self.manifest["archive"] = f"macos/binary/{self.manifest['sha256']}.tar.gz"
        with self.assertRaisesRegex(RuntimeError, "checksum mismatch"):
            INSTALLER["main"]()
        self.assertEqual((app / "Contents/MacOS/termust").read_bytes(), b"previous binary")
        self.assertEqual((self.home / "Applications/Termust.app").resolve(), app.resolve())

    def test_failed_launcher_switch_restores_real_application(self):
        app = self.existing_app()
        with mock.patch.object(INSTALLER["os"], "replace", side_effect=OSError("simulated failure")):
            with self.assertRaisesRegex(OSError, "simulated failure"):
                INSTALLER["main"]()
        self.assertFalse(app.is_symlink())
        self.assertEqual((app / "Contents/MacOS/termust").read_bytes(), b"previous binary")
        INSTALLER["main"]()
        self.assertTrue(app.is_symlink())

    def test_refuses_unmanaged_external_app_link(self):
        outside = self.home / "other-app"
        outside.mkdir()
        destination = self.home / "Applications/Termust.app"
        destination.parent.mkdir()
        destination.symlink_to(outside)
        with self.assertRaisesRegex(RuntimeError, "outside the managed releases"):
            INSTALLER["main"]()
        self.assertEqual(destination.resolve(), outside)

    def test_rejects_traversal(self):
        self.make_archive("Termust.app/../../outside")
        self.manifest["sha256"] = hashlib.sha256(self.archive.read_bytes()).hexdigest()
        self.manifest["archive"] = f"macos/binary/{self.manifest['sha256']}.tar.gz"
        with self.assertRaisesRegex(RuntimeError, "unsafe release archive"):
            INSTALLER["main"]()
        self.assertFalse((self.home / "outside").exists())

    def test_manifest_cannot_redirect_to_another_url_or_platform(self):
        for field, value in (("archive", "https://untrusted.test/app.tar.gz"), ("platform", "linux-x86_64")):
            previous = self.manifest[field]
            self.manifest[field] = value
            with self.assertRaisesRegex(RuntimeError, "invalid macOS release manifest"):
                INSTALLER["main"]()
            self.manifest[field] = previous
        self.assertFalse((self.home / "Applications/Termust.app").exists())

    def test_rejects_unsupported_mac_architecture(self):
        with mock.patch.object(INSTALLER["platform"], "machine", return_value="x86_64"):
            with self.assertRaisesRegex(RuntimeError, "unsupported platform"):
                INSTALLER["main"]()
        self.assertEqual(self.calls, [])


if __name__ == "__main__":
    unittest.main()
