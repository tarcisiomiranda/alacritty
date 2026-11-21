import contextlib
from concurrent.futures import ThreadPoolExecutor
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import plistlib
import runpy
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest import mock


fixtures = runpy.run_path(str(Path(__file__).with_name("test-macos-release.py")))
installer = fixtures["INSTALLER"]
create_archive = fixtures["create_archive"]
download = installer["update_download"]


class UpdaterTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.prefix = self.root / "install"
        self.files = {}
        self.requests = []
        self.platform = ("Linux", "x86_64")
        run = subprocess.run

        def run_command(command, **kwargs):
            if command[0] == "codesign":
                return subprocess.CompletedProcess(command, 0)
            return run(command, **kwargs)

        for patch in (
            mock.patch.dict(installer, update_download=self.fetch, refresh_integrations=mock.Mock(), diagnose=mock.Mock()),
            mock.patch.object(subprocess, "run", side_effect=run_command),
        ):
            patch.start()
            self.addCleanup(patch.stop)

    def fetch(self, relative, path, limit, timeout):
        self.requests.append(relative)
        contents = self.files[relative]
        self.assertLessEqual(len(contents), limit)
        path.write_bytes(contents)

    def invoke(self, *args):
        with mock.patch.object(platform, "system", return_value=self.platform[0]), \
                mock.patch.object(platform, "machine", return_value=self.platform[1]), \
                mock.patch.object(sys, "argv", ["install", "--prefix", str(self.prefix), *map(str, args)]), \
                contextlib.redirect_stdout(io.StringIO()) as output:
            installer["main"]()
        return output.getvalue().splitlines()

    def status(self):
        lines = self.invoke("--update-status")
        self.assertEqual(len(lines), 1)
        return json.loads(lines[0])

    def apply(self, offer):
        return json.loads(self.invoke("--apply-update", offer["id"], "--expected-sha256", offer["sha256"])[-1])

    def linux_release(self, version, stamp):
        release = f"{version}-termust.fixture.{stamp}"
        binary_version = f"termust {version} (fixture-{stamp})"
        binary = f"#!/bin/sh\necho '{binary_version}'\n".encode()
        metadata = {"version": release, "platform": "linux-x86_64", "archive": installer["LINUX_ARCHIVE"],
            "package_version": version, "build_timestamp": stamp, "binary_version": binary_version,
            "binary_sha256": hashlib.sha256(binary).hexdigest()}
        archive = self.root / "linux.tar.gz"
        create_archive(archive, {"termust/VERSION": release.encode(), "termust/bin/termust": binary,
            "termust/release.json": json.dumps(metadata).encode()})
        contents = archive.read_bytes()
        metadata.update(size=len(contents), sha256=hashlib.sha256(contents).hexdigest())
        Path(str(archive) + ".sha256").write_text(metadata["sha256"] + "\n")
        self.files["latest-linux-x86_64.txt"] = release.encode()
        self.files[f"releases/{release}/release.json"] = json.dumps(metadata).encode()
        self.files[f"releases/{release}/{installer['LINUX_ARCHIVE']}"] = contents
        return metadata

    def install_linux(self):
        release = self.linux_release("0.18.0-dev", 100)
        self.invoke("--archive", self.root / "linux.tar.gz", "--no-desktop")
        return release

    def mac_release(self, version, stamp):
        self.platform = ("Darwin", "arm64")
        bundle = {"CFBundleIdentifier": "com.tarcisiomiranda.termust", "CFBundleShortVersionString": version,
            "CFBundleVersion": str(stamp)}
        archive = self.root / "mac.tar.gz"
        create_archive(archive, {"Termust.app/Contents/Info.plist": plistlib.dumps(bundle),
            "Termust.app/Contents/MacOS/termust": f"#!/bin/sh\necho 'termust {version}'\n".encode()})
        contents = archive.read_bytes()
        sha = hashlib.sha256(contents).hexdigest()
        manifest = {"schema": 1, "platform": "macos-arm64", "sha256": sha,
            "archive": f"macos/binary/{sha}.tar.gz", "package_version": version, "build_timestamp": stamp}
        self.files["latest-macos-arm64.json"] = json.dumps(manifest).encode()
        self.files[manifest["archive"]] = contents
        Path(str(archive) + ".sha256").write_text(sha + "\n")
        return manifest

    def install_mac(self):
        self.mac_release("0.18.0-dev", 1)
        self.invoke("--archive", self.root / "mac.tar.gz")
        return (self.prefix / "Termust.app").resolve()

    def test_linux_checks_manifest_without_package_and_preserves_no_desktop(self):
        first = self.install_linux()
        self.assertEqual(self.status(), {"status": "current"})
        self.requests.clear()
        candidate = self.linux_release("0.18.0-dev", 101)
        offer = self.status()
        self.assertEqual(offer, {"status": "available", "version": "0.18.0-dev (1970-01-01 00:01:41 UTC)",
            "id": candidate["version"], "sha256": candidate["sha256"]})
        self.assertEqual(self.requests, ["latest-linux-x86_64.txt", f"releases/{candidate['version']}/release.json"])
        self.assertEqual(self.apply(offer), {"status": "installed"})
        receipt = json.loads((self.prefix / installer["RECEIPT"]).read_bytes())
        self.assertFalse(receipt["desktop"])
        self.assertEqual(receipt["archive_sha256"], candidate["sha256"])
        self.assertFalse((self.prefix / "share/applications/Termust.desktop").exists())
        self.assertNotEqual(first["sha256"], candidate["sha256"])
        self.requests.clear()
        self.assertEqual(self.apply(offer), {"status": "current"})
        self.assertEqual(self.requests, [])
        self.invoke("--rollback")
        self.assertEqual(json.loads((self.prefix / installer["RECEIPT"]).read_bytes())["archive_sha256"], first["sha256"])

    def test_linux_older_versions_equal_timestamps_and_invalid_pointer(self):
        current = self.install_linux()
        rebuild = dict(current, version=current["version"] + ".rebuild", sha256="e" * 64)
        self.files["latest-linux-x86_64.txt"] = rebuild["version"].encode()
        self.files[f"releases/{rebuild['version']}/release.json"] = json.dumps(rebuild).encode()
        self.assertEqual(self.status(), {"status": "current"})
        for version, stamp in (("0.17.9", 110), ("0.18.0-dev", 99)):
            with self.subTest(version=version, stamp=stamp):
                self.linux_release(version, stamp)
                self.assertEqual(self.status(), {"status": "current"})
        self.linux_release("0.19.0", 90)
        self.assertEqual(self.status()["status"], "available")
        self.files["latest-linux-x86_64.txt"] = b"../../other"
        with self.assertRaisesRegex(RuntimeError, "pointer"):
            self.status()

    def test_linux_apply_rechecks_offer_before_downloading_and_rejects_corruption(self):
        first = self.install_linux()
        candidate = self.linux_release("0.18.0-dev", 101)
        offer = self.status()
        self.linux_release("0.18.0-dev", 102)
        with self.assertRaisesRegex(RuntimeError, "release changed"):
            self.apply(offer)
        self.files["latest-linux-x86_64.txt"] = offer["id"].encode()
        manifest_path = f"releases/{offer['id']}/release.json"
        self.files[manifest_path] = json.dumps(dict(candidate, sha256="0" * 64)).encode()
        with self.assertRaisesRegex(RuntimeError, "release changed"):
            self.apply(offer)
        self.files[manifest_path] = json.dumps(candidate).encode()
        archive = f"releases/{offer['id']}/{installer['LINUX_ARCHIVE']}"
        self.files[archive] = bytes(len(self.files[archive]))
        with self.assertRaisesRegex(RuntimeError, "checksum mismatch"):
            self.apply(offer)
        self.assertEqual(json.loads((self.prefix / installer["RECEIPT"]).read_bytes())["version"], first["version"])

    def test_linux_rejects_manifest_disagreement_and_package_path(self):
        self.install_linux()
        candidate = self.linux_release("0.18.0-dev", 102)
        path = f"releases/{candidate['version']}/release.json"
        self.files[path] = json.dumps(dict(candidate, build_timestamp=103)).encode()
        with self.assertRaisesRegex(RuntimeError, "package metadata"):
            self.apply(self.status())
        self.files[path] = json.dumps(dict(candidate, archive="https://elsewhere.example/package")).encode()
        with self.assertRaisesRegex(RuntimeError, "manifest"):
            self.status()

    def test_unmanaged_or_linked_linux_never_contacts_the_feed(self):
        self.assertEqual(self.status(), {"status": "unmanaged"})
        self.assertFalse(self.prefix.exists())
        self.install_linux()
        binary = self.prefix / "bin/termust"
        binary.rename(self.root / "external")
        binary.symlink_to(self.root / "external")
        self.assertEqual(self.status(), {"status": "unmanaged"})
        with self.assertRaisesRegex(RuntimeError, "not managed"):
            self.apply({"id": "fixture", "sha256": "1" * 64})
        self.assertEqual(self.requests, [])

    def test_modified_binary_is_left_in_place_without_contacting_the_feed(self):
        current = self.install_linux()
        binary = self.prefix / "bin/termust"
        binary.write_bytes(b"manually installed executable")
        self.assertEqual(self.status(), {"status": "unmanaged"})
        with self.assertRaisesRegex(RuntimeError, "not managed"):
            self.apply({"id": current["version"], "sha256": current["sha256"]})
        self.assertEqual(self.requests, [])
        self.assertEqual(binary.read_bytes(), b"manually installed executable")

    def test_concurrent_apply_observes_the_first_install_under_the_shared_lock(self):
        self.install_linux()
        candidate = self.linux_release("0.18.0-dev", 101)
        offer = self.status()
        original = installer["install_lock"]
        waiting = threading.Event()

        def competing_lock(prefix, wait=0):
            waiting.set()
            return original(prefix, wait=wait)

        with ThreadPoolExecutor(max_workers=1) as pool, mock.patch.dict(installer, install_lock=competing_lock):
            with original(self.prefix):
                second = pool.submit(self.apply, offer)
                self.assertTrue(waiting.wait(3))
                local = installer["managed_update"](self.prefix, "linux-x86_64")
                with tempfile.TemporaryDirectory(dir=self.prefix) as stage:
                    first = installer["check_update"](self.prefix, "linux-x86_64", local, Path(stage),
                        (offer["id"], offer["sha256"]))
                    self.assertEqual(first, {"status": "installed"})
            self.assertEqual(second.result(timeout=5), {"status": "current"})
        self.assertEqual(self.requests.count(f"releases/{candidate['version']}/{installer['LINUX_ARCHIVE']}"), 1)

    def test_mac_old_feed_downgrade_and_updated_launcher(self):
        original = self.install_mac()
        candidate = self.mac_release("0.18.0-dev", 102)
        old_feed = dict(candidate)
        for key in ("package_version", "build_timestamp"):
            del old_feed[key]
        self.files["latest-macos-arm64.json"] = json.dumps(old_feed).encode()
        self.assertEqual(self.status(), {"status": "current"})
        self.mac_release("0.17.0", 200)
        self.assertEqual(self.status(), {"status": "current"})
        candidate = self.mac_release("0.18.0-dev", 102)
        offer = self.status()
        self.assertEqual((offer["id"], offer["sha256"]), (candidate["sha256"], candidate["sha256"]))
        self.assertEqual(self.apply(offer), {"status": "installed"})
        self.assertEqual((self.prefix / "Termust.app").resolve().parent.name, offer["id"])
        self.assertEqual((self.prefix / ".termust/previous.app").resolve(), original)
        self.assertEqual(self.apply(offer), {"status": "current"})

    def test_mac_refuses_external_or_unlinked_application_before_network(self):
        app = self.install_mac()
        launcher = self.prefix / "Termust.app"
        launcher.unlink()
        external = self.root / "Termust.app"
        app.rename(external)
        launcher.symlink_to(external)
        self.assertEqual(self.status(), {"status": "unmanaged"})
        launcher.unlink()
        external.rename(launcher)
        self.assertEqual(self.status(), {"status": "unmanaged"})
        self.assertEqual(self.requests, [])

    def test_mac_rejects_changed_version_and_archive_without_replacing_launcher(self):
        previous = self.install_mac()
        candidate = self.mac_release("0.18.0-dev", 102)
        offer = self.status()
        self.files["latest-macos-arm64.json"] = json.dumps(dict(candidate, build_timestamp=103)).encode()
        with self.assertRaisesRegex(RuntimeError, "package version"):
            self.apply(offer)
        self.files[candidate["archive"]] = b"corrupt"
        with self.assertRaisesRegex(RuntimeError, "checksum mismatch"):
            self.apply(offer)
        self.files["latest-macos-arm64.json"] = json.dumps(dict(candidate, archive="https://example.com/app")).encode()
        with self.assertRaisesRegex(RuntimeError, "manifest"):
            self.status()
        self.assertEqual((self.prefix / "Termust.app").resolve(), previous)

    def test_update_manifests_reject_dates_outside_the_supported_calendar(self):
        self.install_linux()
        self.linux_release("0.18.0-dev", 253402300800)
        with self.assertRaisesRegex(RuntimeError, "manifest"):
            self.status()
        self.prefix = self.root / "Applications"
        self.install_mac()
        self.mac_release("0.19.0", 253402300800)
        with self.assertRaisesRegex(RuntimeError, "manifest"):
            self.status()

    def test_embedded_mode_rejects_overrides(self):
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            self.invoke("--update-status", "--base-url", "https://elsewhere.example")
        self.assertEqual(self.requests, [])

    def test_transport_rejects_redirects_and_limits_streamed_bytes(self):
        executable = self.root / "curl"
        executable.write_text("#!/bin/sh\nprintf '%s' \"$RESPONSE\"\n")
        executable.chmod(0o755)
        dest = self.root / "result"
        for response, error in (("abc\n200", None), ("abc\n302", "HTTP response"), ("toolong\n200", "size limit")):
            with self.subTest(response=response), mock.patch.dict(os.environ, PATH=str(self.root), RESPONSE=response):
                if error:
                    with self.assertRaisesRegex(RuntimeError, error):
                        download("latest.txt", dest, 3, 1)
                else:
                    download("latest.txt", dest, 3, 1)
                    self.assertEqual(dest.read_bytes(), b"abc")


if __name__ == "__main__":
    unittest.main()
