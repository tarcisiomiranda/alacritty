import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("termust_publisher", ROOT / "scripts/publish-b2.py")
PUBLISHER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PUBLISHER)


class PublicationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="termust-release-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.dist = self.root / "dist"
        self.dist.mkdir()
        self.addCleanup(mock.patch.stopall)
        mock.patch.object(PUBLISHER, "ROOT", self.root).start()
        mock.patch.object(PUBLISHER, "DIST", self.dist).start()
        self.config = {"bucket": "fixture", "base_url": "https://downloads.example.test"}
        self.archive_name = "termust-linux-x86_64.tar.gz"
        self.archive = b"fixture archive"
        self.installer = b"fixture installer"
        self.manifest = {
            "version": "0.18.0-termust.fixture", "platform": "linux-x86_64", "portable": True,
            "minimum_glibc": "2.29", "archive": self.archive_name,
            "sha256": hashlib.sha256(self.archive).hexdigest(),
            "installer_sha256": hashlib.sha256(self.installer).hexdigest(),
            "binary_sha256": hashlib.sha256(b"fixture binary").hexdigest(),
            "binary_version": "termust 0.18.0-dev (fixture)", "build_os": "Debian GNU/Linux 11",
        }
        self.report = {
            "schema": 1, "archive_sha256": self.manifest["sha256"],
            "installer_sha256": self.manifest["installer_sha256"],
            "results": [{"distro": distro, "status": "passed", "version": self.manifest["version"],
                         "binary_sha256": self.manifest["binary_sha256"],
                         "checks": sorted(PUBLISHER.REQUIRED_CHECKS)} for distro in sorted(PUBLISHER.REQUIRED_DISTROS)],
        }
        (self.root / "install.sh").write_bytes(self.installer)
        (self.dist / self.archive_name).write_bytes(self.archive)
        (self.dist / (self.archive_name + ".sha256")).write_text(self.manifest["sha256"] + "  " + self.archive_name + "\n")
        (self.dist / "latest-linux-x86_64.txt").write_text(self.manifest["version"] + "\n")
        self.save()

    def save(self):
        (self.dist / "release.json").write_text(json.dumps(self.manifest))
        (self.dist / "compatibility.json").write_text(json.dumps(self.report))

    def assert_rejected(self, message):
        with mock.patch.object(PUBLISHER, "b2") as b2, mock.patch.object(PUBLISHER, "put") as put, mock.patch.object(PUBLISHER, "download") as download:
            with self.assertRaisesRegex((RuntimeError, ValueError, OSError), message):
                PUBLISHER.publish(self.config)
            b2.assert_not_called()
            put.assert_not_called()
            download.assert_not_called()

    def remote_objects(self):
        prefix = "releases/" + self.manifest["version"] + "/"
        return {
            "latest-linux-x86_64.txt": self.manifest["version"].encode(),
            prefix + "release.json": json.dumps(self.manifest).encode(),
            prefix + self.archive_name: self.archive,
            prefix + self.archive_name + ".sha256": (self.manifest["sha256"] + "  " + self.archive_name).encode(),
            prefix + "install.sh": self.installer, "install.sh": self.installer,
            prefix + "compatibility.json": json.dumps(self.report).encode(),
        }

    def test_valid_inputs_and_immutable_report_before_aliases(self):
        self.assertEqual(PUBLISHER.publication_inputs(), self.manifest)
        with mock.patch.object(PUBLISHER, "b2", return_value=[{"bucketName": "fixture", "bucketType": "allPublic"}]), mock.patch.object(PUBLISHER, "put") as put, mock.patch.object(PUBLISHER, "verify") as verify:
            PUBLISHER.publish(self.config)
        names = [call.args[2] for call in put.call_args_list]
        report_name = "releases/" + self.manifest["version"] + "/compatibility.json"
        report_call = next(call for call in put.call_args_list if call.args[2] == report_name)
        self.assertTrue(report_call.kwargs["immutable"])
        self.assertLess(names.index(report_name), names.index("install.sh"))
        self.assertEqual(names[-1], "latest-linux-x86_64.txt")
        verify.assert_called_once_with(self.config)

    def test_legacy_mirror_is_published_only_after_primary_verification(self):
        self.config.update(legacy_bucket="legacy", legacy_base_url="https://legacy.example.test")
        events = []
        buckets = [{"bucketName": name, "bucketType": "allPublic"} for name in ("fixture", "legacy")]
        with mock.patch.object(PUBLISHER, "b2", return_value=buckets), mock.patch.object(PUBLISHER, "put", side_effect=lambda config, path, name, *args, **kwargs: events.append(("put", config["bucket"], name))), mock.patch.object(PUBLISHER, "verify", side_effect=lambda config: events.append(("verify", config["bucket"]))):
            PUBLISHER.publish(self.config)
        first_legacy = next(index for index, event in enumerate(events) if event[1] == "legacy")
        self.assertLess(events.index(("verify", "fixture")), first_legacy)
        for bucket in ("fixture", "legacy"):
            self.assertIn(("put", bucket, "install.sh"), events)
            self.assertIn(("put", bucket, "latest-linux-x86_64.txt"), events)
            self.assertIn(("verify", bucket), events)

    def test_multiple_mirrors_receive_the_verified_release(self):
        self.config["mirrors"] = [{"bucket": name, "base_url": "https://" + name + ".example.test"} for name in ("previous", "original")]
        events = []
        buckets = [{"bucketName": name, "bucketType": "allPublic"} for name in ("fixture", "previous", "original")]
        with mock.patch.object(PUBLISHER, "b2", return_value=buckets), mock.patch.object(PUBLISHER, "put", side_effect=lambda config, path, name, *args, **kwargs: events.append(("put", config["bucket"], name))), mock.patch.object(PUBLISHER, "verify", side_effect=lambda config: events.append(("verify", config["bucket"]))):
            PUBLISHER.publish(self.config)
        primary_verified = events.index(("verify", "fixture"))
        for bucket in ("previous", "original"):
            first_write = next(i for i, event in enumerate(events) if event[:2] == ("put", bucket))
            self.assertGreater(first_write, primary_verified)
            self.assertIn(("put", bucket, "install.sh"), events)
            self.assertIn(("put", bucket, "latest-linux-x86_64.txt"), events)
            self.assertIn(("verify", bucket), events)

    def test_invalid_or_duplicate_mirror_list_is_rejected_before_publication(self):
        for targets in ({"0": {}}, [{"bucket": "fixture", "base_url": "https://example.test"}], [{"bucket": "previous", "base_url": "http://example.test"}], [{"bucket": "previous", "base_url": "https://example.test"}] * 2):
            with self.subTest(targets=targets):
                self.config["mirrors"] = targets
                self.assert_rejected("legacy mirror")

    def test_primary_verification_failure_does_not_change_legacy_mirror(self):
        self.config.update(legacy_bucket="legacy", legacy_base_url="https://legacy.example.test")
        buckets = [{"bucketName": name, "bucketType": "allPublic"} for name in ("fixture", "legacy")]
        with mock.patch.object(PUBLISHER, "b2", return_value=buckets), mock.patch.object(PUBLISHER, "put") as put, mock.patch.object(PUBLISHER, "verify", side_effect=RuntimeError("primary verification failed")):
            with self.assertRaisesRegex(RuntimeError, "primary verification failed"):
                PUBLISHER.publish(self.config)
        self.assertTrue(put.call_args_list)
        self.assertTrue(all(call.args[0]["bucket"] == "fixture" for call in put.call_args_list))

    def test_incomplete_or_unsafe_legacy_mirror_settings_are_rejected(self):
        for fields in ("legacy_bucket = 'legacy'", "legacy_bucket = 'fixture'\nlegacy_base_url = 'https://legacy.example.test'", "legacy_bucket = 'legacy'\nlegacy_base_url = 'http://legacy.example.test'"):
            with self.subTest(fields=fields):
                (self.root / "distribution.toml").write_text("bucket = 'fixture'\nbase_url = 'https://downloads.example.test'\n" + fields)
                with self.assertRaisesRegex(RuntimeError, "legacy mirror"):
                    PUBLISHER.settings()

    def test_native_or_missing_portable_flag_is_rejected(self):
        for portable in (False, None, 1, "true"):
            with self.subTest(portable=portable):
                self.manifest["portable"] = portable
                self.save()
                self.assert_rejected("portable build")

    def test_invalid_or_newer_glibc_is_rejected(self):
        for minimum in ("2.43", "2.32", "3.0", "2.31.0", None, [], "unknown"):
            with self.subTest(minimum=minimum):
                self.manifest["minimum_glibc"] = minimum
                self.save()
                self.assert_rejected("glibc 2.31")

    def test_archive_or_installer_changes_are_rejected(self):
        for path, original, message in (
            (self.dist / self.archive_name, self.archive, "local archive"),
            (self.root / "install.sh", self.installer, "installer changed"),
        ):
            with self.subTest(path=path.name):
                path.write_bytes(original + b"changed")
                self.assert_rejected(message)
                path.write_bytes(original)

    def test_bad_checksum_or_latest_pointer_rejected_before_b2(self):
        for path, value, message in (
            (self.dist / (self.archive_name + ".sha256"), "0" * 64, "checksum file"),
            (self.dist / "latest-linux-x86_64.txt", "another-release", "latest pointer"),
        ):
            with self.subTest(path=path.name):
                original = path.read_text()
                path.write_text(value)
                self.assert_rejected(message)
                path.write_text(original)

    def test_missing_or_invalid_report_is_rejected(self):
        path = self.dist / "compatibility.json"
        path.unlink()
        self.assert_rejected("report is missing")
        for data in ("{broken", "null", "[]", '{"schema":true}', '{"schema":2}'):
            with self.subTest(data=data):
                path.write_text(data)
                self.assert_rejected("Expecting|schema 1")

    def test_report_hashes_must_match_packaged_inputs(self):
        for field in ("archive_sha256", "installer_sha256"):
            with self.subTest(field=field):
                original = self.report[field]
                self.report[field] = "0" * 64
                self.save()
                self.assert_rejected("report is stale")
                self.report[field] = original

    def test_partial_failed_duplicate_and_unknown_distros_are_rejected(self):
        original = copy.deepcopy(self.report)
        for mutation, message in (
            (lambda r: r["results"].pop(), "all six"),
            (lambda r: r["results"][0].update(status="failed"), "did not pass"),
            (lambda r: r["results"][0].update(status="skipped"), "did not pass"),
            (lambda r: r["results"][0].update(distro=r["results"][1]["distro"]), "duplicate"),
            (lambda r: r["results"][0].update(distro="unknown"), "invalid"),
            (lambda r: r["results"][0].update(distro=[]), "invalid"),
        ):
            with self.subTest(message=message):
                self.report = copy.deepcopy(original)
                mutation(self.report)
                self.save()
                self.assert_rejected(message)

    def test_result_release_and_graphical_coverage_are_checked(self):
        original = copy.deepcopy(self.report)
        for change, message in (
            ({"version": "old-release"}, "different release"),
            ({"binary_sha256": "0" * 64}, "different release"),
            ({"checks": ["install", "version"]}, "graphical checks"),
            ({"checks": None}, "graphical checks"),
        ):
            with self.subTest(change=change):
                self.report = copy.deepcopy(original)
                self.report["results"][0].update(change)
                self.save()
                self.assert_rejected(message)

    def test_public_portable_verification_checks_report(self):
        objects = self.remote_objects()
        with mock.patch.object(PUBLISHER, "download", side_effect=lambda config, name: objects[name]) as download:
            self.assertEqual(PUBLISHER.verify(self.config), self.manifest)
        self.assertIn("releases/" + self.manifest["version"] + "/compatibility.json", [call.args[1] for call in download.call_args_list])
        self.report["results"][0]["status"] = "failed"
        objects = self.remote_objects()
        with mock.patch.object(PUBLISHER, "download", side_effect=lambda config, name: objects[name]):
            with self.assertRaisesRegex(RuntimeError, "did not pass"):
                PUBLISHER.verify(self.config)

    def test_legacy_public_release_verifies_without_report(self):
        del self.manifest["portable"]
        self.manifest["minimum_glibc"] = "2.43"
        objects = self.remote_objects()
        del objects["releases/" + self.manifest["version"] + "/compatibility.json"]
        with mock.patch.object(PUBLISHER, "download", side_effect=lambda config, name: objects[name]):
            self.assertEqual(PUBLISHER.verify(self.config), self.manifest)

    def test_standalone_installer_update_reaches_all_common_urls(self):
        spec = importlib.util.spec_from_file_location("standalone_installer", ROOT / "scripts/publish-installer.py")
        standalone = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(standalone)
        self.config["mirrors"] = [
            {"bucket": "previous", "base_url": "https://previous.example.test"},
            {"bucket": "original", "base_url": "https://original.example.test"},
        ]
        mac_hash = hashlib.sha256(self.archive).hexdigest()
        mac_name = f"macos/binary/{mac_hash}.tar.gz"
        mac_manifest = {"schema": 1, "platform": "macos-arm64", "sha256": mac_hash, "archive": mac_name}
        objects = {
            ("fixture", mac_name): self.archive,
            ("fixture", "latest-macos-arm64.json"): json.dumps(mac_manifest).encode(),
        }
        uploads = []

        def put(target, path, name, content_type):
            key = (target["bucket"], name)
            objects[key] = path.read_bytes()
            uploads.append(key)

        with (
            mock.patch.object(standalone, "ROOT", self.root),
            mock.patch.object(standalone, "publisher", PUBLISHER),
            mock.patch.object(PUBLISHER, "settings", return_value=self.config),
            mock.patch.object(PUBLISHER, "put", side_effect=put),
            mock.patch.object(PUBLISHER, "download", side_effect=lambda target, name: objects[target["bucket"], name]),
        ):
            standalone.main()
        checksum = hashlib.sha256(self.installer).hexdigest()
        for bucket in ("fixture", "previous", "original"):
            self.assertEqual(objects[bucket, "install.sh"], self.installer)
            self.assertEqual(objects[bucket, "install.sh.sha256"].decode().split(), [checksum, "install.sh"])
        self.assertEqual(uploads, [
            ("fixture", "install.sh.sha256"), ("fixture", "install.sh"), ("fixture", "install-macos.sh"),
            ("previous", "install.sh.sha256"), ("previous", "install.sh"),
            ("original", "install.sh.sha256"), ("original", "install.sh"),
        ])

    def test_common_installer_can_evolve_without_changing_pinned_releases(self):
        objects = self.remote_objects()
        objects["install.sh"] = b"common installer with macOS support"
        sha = hashlib.sha256(objects["install.sh"]).hexdigest()
        objects["install.sh.sha256"] = (sha + "  install.sh\n").encode()
        with mock.patch.object(PUBLISHER, "download", side_effect=lambda config, name: objects[name]):
            self.assertEqual(PUBLISHER.verify(self.config), self.manifest)
            objects["install.sh"] += b"unexpected change"
            with self.assertRaisesRegex(RuntimeError, "common installer checksum mismatch"):
                PUBLISHER.verify(self.config)

    def test_pinned_installer_still_requires_its_original_hash(self):
        objects = self.remote_objects()
        objects["releases/" + self.manifest["version"] + "/install.sh"] += b"changed"
        with mock.patch.object(PUBLISHER, "download", side_effect=lambda config, name: objects[name]):
            with self.assertRaisesRegex(RuntimeError, "pinned installer"):
                PUBLISHER.verify(self.config)


if __name__ == "__main__":
    unittest.main(verbosity=2)
