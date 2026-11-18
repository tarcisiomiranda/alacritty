import argparse
import hashlib
import json
import re
import subprocess
import tomllib
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DIST = ROOT / "dist"


def settings():
    data = tomllib.loads((ROOT / "distribution.toml").read_text())
    parsed = urllib.parse.urlsplit(data["base_url"])
    if parsed.scheme != "https" or parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise RuntimeError("The public release URL must be a plain HTTPS URL.")
    mirror_targets(data)
    return data


def mirror_targets(config):
    targets = config.get("mirrors", [])
    if not isinstance(targets, list):
        raise RuntimeError("The legacy mirrors must be a list of bucket and URL pairs.")
    targets = list(targets)
    legacy = (config.get("legacy_bucket"), config.get("legacy_base_url"))
    if any(legacy):
        targets.append({"bucket": legacy[0], "base_url": legacy[1]})
    seen = {config["bucket"]}
    result = []
    for target in targets:
        if not isinstance(target, dict) or not all(isinstance(target.get(key), str) and target[key] for key in ("bucket", "base_url")) or target["bucket"] in seen:
            raise RuntimeError("Each legacy mirror needs a separate unique bucket and download URL.")
        parsed = urllib.parse.urlsplit(target["base_url"])
        if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password or parsed.query or parsed.fragment or any(ord(c) <= 32 or ord(c) == 127 for c in target["base_url"]):
            raise RuntimeError("The legacy mirror URL must be a plain HTTPS URL.")
        seen.add(target["bucket"])
        result.append({"bucket": target["bucket"], "base_url": target["base_url"]})
    return result


def b2(*args, as_json=False):
    try:
        result = subprocess.run(["b2", *args], capture_output=True, text=True, timeout=600)
    except FileNotFoundError:
        raise RuntimeError("B2 CLI 5 is required to publish. Use its existing account authorization.") from None
    except subprocess.TimeoutExpired:
        raise RuntimeError("B2 command timed out. Verify the public release before retrying.") from None
    if result.returncode:
        raise RuntimeError(f"B2 {args[0]} {args[1]} failed (exit {result.returncode}). Check the CLI account and permissions locally.")
    return json.loads(result.stdout) if as_json else None


def download(config, name, missing_ok=False):
    url = config["base_url"].rstrip("/") + "/" + urllib.parse.quote(name, safe="/-_.+")
    request = urllib.request.Request(url, headers={"Cache-Control": "no-cache"})
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            return response.read()
    except urllib.error.HTTPError as error:
        if missing_ok and error.code == 404:
            return None
        raise RuntimeError(f"Public download failed for {name}: HTTP {error.code}") from None
    except urllib.error.URLError as error:
        raise RuntimeError(f"Public download failed for {name}: {error.reason}") from None


def version_id(value):
    if not isinstance(value, str) or value in (".", "..") or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._+-]{0,199}", value):
        raise RuntimeError("Invalid release version.")
    return value


def put(config, path, name, content_type, immutable=False):
    data = path.read_bytes()
    existing = download(config, name, missing_ok=True)
    if existing is not None:
        if existing == data:
            print(f"Already verified: {name}")
            return
        if immutable:
            raise RuntimeError(f"Release already exists with different content: {name}")
    cache = "public, max-age=31536000, immutable" if immutable else "no-cache, no-store, max-age=0"
    b2("file", "upload", "--no-progress", "--content-type", content_type,
       "--cache-control", cache, config["bucket"], str(path), name)
    if download(config, name) != data:
        raise RuntimeError(f"Uploaded bytes did not match the public download: {name}")
    print(f"Published and verified: {name}")


REQUIRED_DISTROS = frozenset(("ubuntu-22.04", "ubuntu-24.04", "debian-12", "debian-13", "fedora-43", "fedora-44"))
REQUIRED_CHECKS = frozenset(("install", "version", "receipt", "check", "reinstall", "path",
                             "desktop-entry", "terminfo", "graphical-pty", "desktop-launch-pty"))


def portable_manifest(manifest):
    if not isinstance(manifest, dict) or manifest.get("portable") is not True:
        raise RuntimeError("Public releases must use a portable build. Run mise run package:portable.")
    minimum = manifest.get("minimum_glibc")
    if not isinstance(minimum, str) or not re.fullmatch(r"[0-9]+\.[0-9]+", minimum) or tuple(map(int, minimum.split("."))) > (2, 31):
        raise RuntimeError("Public releases must require glibc 2.31 or older.")
    if manifest.get("platform") != "linux-x86_64" or manifest.get("archive") != "termust-linux-x86_64.tar.gz":
        raise RuntimeError("Portable release platform or archive name is invalid.")
    for key in ("sha256", "installer_sha256", "binary_sha256"):
        if not isinstance(manifest.get(key), str) or not re.fullmatch(r"[0-9a-f]{64}", manifest[key]):
            raise RuntimeError(f"Portable release metadata has an invalid {key}.")
    version_id(manifest.get("version"))


def compatibility_report(manifest, report):
    if not isinstance(report, dict) or type(report.get("schema")) is not int or report["schema"] != 1:
        raise RuntimeError("Compatibility report must use schema 1. Run mise run test:distros.")
    if report.get("archive_sha256") != manifest["sha256"] or report.get("installer_sha256") != manifest["installer_sha256"]:
        raise RuntimeError("Compatibility report is stale: archive or installer changed. Run mise run test:distros.")
    results = report.get("results")
    if not isinstance(results, list) or len(results) != len(REQUIRED_DISTROS):
        raise RuntimeError("Compatibility report must contain all six distributions. Run mise run test:distros.")
    seen = set()
    for result in results:
        if not isinstance(result, dict) or not isinstance(result.get("distro"), str) or result["distro"] not in REQUIRED_DISTROS or result["distro"] in seen:
            raise RuntimeError("Compatibility report has an invalid or duplicate distribution.")
        seen.add(result["distro"])
        if result.get("status") != "passed":
            raise RuntimeError(f"Compatibility tests did not pass for {result['distro']}.")
        if result.get("version") != manifest["version"] or result.get("binary_sha256") != manifest["binary_sha256"]:
            raise RuntimeError(f"Compatibility result belongs to a different release: {result['distro']}.")
        checks = result.get("checks")
        if not isinstance(checks, list) or any(not isinstance(check, str) for check in checks) or not REQUIRED_CHECKS.issubset(checks):
            raise RuntimeError(f"Compatibility result lacks required install and graphical checks: {result['distro']}.")


def publication_inputs():
    manifest = json.loads((DIST / "release.json").read_text())
    portable_manifest(manifest)
    installer = ROOT / "install.sh"
    archive = DIST / manifest["archive"]
    if hashlib.sha256(installer.read_bytes()).hexdigest() != manifest["installer_sha256"]:
        raise RuntimeError("The installer changed after packaging. Run mise run package:portable again.")
    if hashlib.sha256(archive.read_bytes()).hexdigest() != manifest["sha256"]:
        raise RuntimeError("The local archive does not match the release metadata.")
    checksum = (DIST / (manifest["archive"] + ".sha256")).read_text().split()
    if checksum != [manifest["sha256"], manifest["archive"]]:
        raise RuntimeError("The local archive checksum file does not match the release metadata.")
    if (DIST / "latest-linux-x86_64.txt").read_text().strip() != manifest["version"]:
        raise RuntimeError("Local latest pointer does not match the packaged release.")
    report_path = DIST / "compatibility.json"
    if not report_path.is_file():
        raise RuntimeError("Compatibility report is missing. Run mise run test:distros before publishing.")
    compatibility_report(manifest, json.loads(report_path.read_text()))
    return manifest



def verify(config):
    version = version_id(download(config, "latest-linux-x86_64.txt").decode().strip())
    prefix = f"releases/{version}/"
    manifest = json.loads(download(config, prefix + "release.json"))
    if manifest.get("version") != version or manifest.get("platform") != "linux-x86_64":
        raise RuntimeError("Public release metadata does not match the latest pointer.")
    archive_name = "termust-linux-x86_64.tar.gz"
    if manifest.get("archive") != archive_name or not re.fullmatch(r"[0-9a-f]{64}", manifest.get("sha256", "")):
        raise RuntimeError("Public archive metadata is invalid.")
    archive = download(config, prefix + archive_name)
    checksum = download(config, prefix + archive_name + ".sha256").decode().split()
    if checksum != [manifest["sha256"], archive_name] or hashlib.sha256(archive).hexdigest() != manifest["sha256"]:
        raise RuntimeError("Public release failed SHA256 verification.")
    pinned = download(config, prefix + "install.sh")
    current = download(config, "install.sh")
    expected = manifest.get("installer_sha256")
    if hashlib.sha256(pinned).hexdigest() != expected:
        raise RuntimeError("Public pinned installer does not match release metadata.")
    current_hash = hashlib.sha256(current).hexdigest()
    if current_hash != expected:
        current_checksum = download(config, "install.sh.sha256").decode().split()
        if current_checksum != [current_hash, "install.sh"]:
            raise RuntimeError("Public common installer checksum mismatch.")
    if manifest.get("portable") is True:
        portable_manifest(manifest)
        report = json.loads(download(config, prefix + "compatibility.json"))
        compatibility_report(manifest, report)
        print("Compatibility verified: all six distributions passed.")
    print(f"Public release verified: {version}")
    print(f"Binary: {manifest['binary_version']}")
    print(f"Built on: {manifest['build_os']}; minimum glibc: {manifest['minimum_glibc']}")
    print(f"Install: curl -fsSL {config['base_url']}/install.sh | bash")
    return manifest


def publish(config):
    mirrors = mirror_targets(config)
    manifest = publication_inputs()
    version = manifest["version"]
    installer = ROOT / "install.sh"
    buckets = b2("bucket", "list", "--json", as_json=True)
    bucket = next((item for item in buckets if item.get("bucketName") == config["bucket"]), None)
    if bucket is None:
        b2("bucket", "create", "--lifecycle-rule",
           '{"fileNamePrefix":"","daysFromHidingToDeleting":7,"daysFromUploadingToHiding":null}',
           config["bucket"], "allPublic")
        print(f"Created dedicated public bucket: {config['bucket']}")
    elif bucket.get("bucketType") != "allPublic":
        raise RuntimeError("The target bucket is private. Refusing to change an existing bucket's access.")
    archive_name = manifest["archive"]
    prefix = f"releases/{version}/"
    for filename, content_type in [(archive_name, "application/gzip"),
                                   (archive_name + ".sha256", "text/plain; charset=utf-8"),
                                   ("release.json", "application/json"),
                                   ("compatibility.json", "application/json")]:
        put(config, DIST / filename, prefix + filename, content_type, immutable=True)
    put(config, installer, prefix + "install.sh", "text/x-shellscript; charset=utf-8", immutable=True)
    installer_checksum = DIST / "install.sh.sha256"
    installer_checksum.write_text(hashlib.sha256(installer.read_bytes()).hexdigest() + "  install.sh\n")
    put(config, installer_checksum, "install.sh.sha256", "text/plain; charset=utf-8")
    put(config, installer, "install.sh", "text/x-shellscript; charset=utf-8")
    pointer = DIST / "latest-linux-x86_64.txt"
    put(config, pointer, "latest-linux-x86_64.txt", "text/plain; charset=utf-8")
    verify(config)
    for mirror in mirrors:
        print("Publishing the verified release to legacy mirror: " + mirror["bucket"])
        publish(mirror)


def main():
    parser = argparse.ArgumentParser(description="Publish and verify Termust packages in B2.")
    parser.add_argument("action", choices=["publish", "verify"])
    args = parser.parse_args()
    try:
        if args.action == "publish":
            publish(settings())
        else:
            verify(settings())
    except (RuntimeError, OSError, ValueError) as error:
        raise SystemExit(str(error)) from None


if __name__ == "__main__":
    main()
