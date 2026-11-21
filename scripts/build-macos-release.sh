#!/usr/bin/env bash
set -eu

if ! command -v python3 >/dev/null 2>&1; then
    printf '%s\n' 'Error: python3 is required. Install Xcode Command Line Tools first.' >&2
    exit 1
fi

python3 - "$@" <<'TERMUST_PYTHON'
import gzip
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import plistlib
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time

BASE_URL = "https://f005.backblazeb2.com/file/termust"
BUCKET = "termust"
SOURCE_SHA256 = "@SOURCE_SHA256@"


def run(command, **kwargs):
    return subprocess.run(command, check=True, **kwargs)


def download(name, target):
    run(["curl", "--fail", "--silent", "--show-error", "--location", "--proto", "=https", "--proto-redir", "=https", "--tlsv1.2", "--retry", "3", "--connect-timeout", "15", "--max-time", "600", "--output", str(target), f"{BASE_URL}/{name}"])


def unpack(archive, root):
    with tarfile.open(archive, "r:gz") as package:
        members = package.getmembers()
        if len(members) > 30000 or sum(member.size for member in members) > 1024 ** 3:
            raise RuntimeError("source archive is too large.")
        seen = set()
        for member in members:
            path = PurePosixPath(member.name)
            if (not path.parts or path.parts[0] != "termust-source" or path.is_absolute() or ".." in path.parts
                    or str(path) in seen or not (member.isdir() or member.isfile())
                    or any(ord(c) < 32 or ord(c) == 127 for c in member.name)):
                raise RuntimeError("unsafe source archive entry.")
            seen.add(str(path))
        for member in members:
            target = root.joinpath(*PurePosixPath(member.name).parts)
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                with package.extractfile(member) as source, target.open("xb") as output:
                    shutil.copyfileobj(source, output)
                target.chmod(0o755 if member.mode & 0o111 else 0o644)


def rust_command():
    candidates = [["mise", "exec", "rust@1.89.0", "--"]] if shutil.which("mise") else []
    candidates.append([])
    errors = []
    for prefix in candidates:
        try:
            result = subprocess.run([*prefix, "rustc", "-vV"], text=True, capture_output=True)
        except OSError as error:
            errors.append(str(error))
            continue
        if result.returncode == 0 and "host: aarch64-apple-darwin" in result.stdout.splitlines():
            return prefix
        errors.append(result.stderr.strip() or "Rust is not native Apple Silicon")
    raise RuntimeError("Native Rust is unavailable. Install Rust with mise or rustup.\n" + "\n".join(errors))


def archive_app(app, archive):
    with archive.open("wb") as raw, gzip.GzipFile(fileobj=raw, mode="wb", mtime=0, filename="") as compressed, tarfile.open(fileobj=compressed, mode="w") as tar:
        for path in [app, *sorted(app.rglob("*"))]:
            if path.is_symlink() or not (path.is_dir() or path.is_file()):
                raise RuntimeError(f"unsupported bundle entry: {path}")
            info = tar.gettarinfo(str(path), arcname=str(path.relative_to(app.parent)))
            info.uid = info.gid = 0
            info.uname = info.gname = ""
            info.mtime = 0
            if path.is_file():
                with path.open("rb") as data:
                    tar.addfile(info, data)
            else:
                tar.addfile(info)


def main():
    if platform.system() != "Darwin" or platform.machine() != "arm64":
        raise RuntimeError("build this release on a native Apple Silicon Mac.")
    if len(sys.argv) not in (1, 3) or (len(sys.argv) == 3 and sys.argv[1] != "--source-dir"):
        raise RuntimeError("usage: build-macos-release.sh [--source-dir PROJECT_DIRECTORY]")
    for tool in ("curl", "b2", "codesign", "xcrun"):
        if not shutil.which(tool):
            raise RuntimeError(f"{tool} is required. Install Xcode Command Line Tools and the B2 CLI, then authorize B2 with 'b2 account authorize'.")
    prefix = rust_command()
    run(["xcrun", "--find", "clang"], stdout=subprocess.DEVNULL)
    with tempfile.TemporaryDirectory(prefix="termust-macos-release-") as temporary:
        root = Path(temporary)
        if len(sys.argv) == 3:
            source = Path(sys.argv[2]).expanduser().resolve()
            if not (source / "Cargo.lock").is_file() or not (source / "install.sh").is_file():
                raise RuntimeError("--source-dir must point to a complete Termust project.")
        else:
            source_archive = root / "source.tar.gz"
            download(f"macos/source/{SOURCE_SHA256}.tar.gz", source_archive)
            if hashlib.sha256(source_archive.read_bytes()).hexdigest() != SOURCE_SHA256:
                raise RuntimeError("source archive checksum mismatch.")
            unpack(source_archive, root)
            source = root / "termust-source"
        env = dict(os.environ, CARGO_TARGET_DIR=str(root / "target"))
        print("Building Termust for macOS Apple Silicon; the first build can take several minutes.", flush=True)
        run([*prefix, "cargo", "build", "--locked", "--release", "--target", "aarch64-apple-darwin", "-p", "termust"], cwd=source, env=env)
        executable = root / "target/aarch64-apple-darwin/release/termust"
        version_output = run([str(executable), "--version"], text=True, stdout=subprocess.PIPE).stdout.strip()
        version_parts = version_output.split()
        if len(version_parts) < 2 or version_parts[0] != "termust":
            raise RuntimeError("could not determine the Termust package version from the built executable.")
        package_version = version_parts[1]
        print(version_output)
        app = root / "Termust.app"
        shutil.copytree(source / "extra/osx/Termust.app", app, symlinks=False)
        build_timestamp = int(time.time())
        plist = app / "Contents/Info.plist"
        info = plistlib.loads(plist.read_bytes())
        info["CFBundleShortVersionString"] = package_version
        info["CFBundleVersion"] = str(build_timestamp)
        plist.write_bytes(plistlib.dumps(info, sort_keys=False))
        binary = app / "Contents/MacOS/termust"
        binary.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(executable, binary)
        for license_name in ("LICENSE-APACHE", "LICENSE-MIT"):
            shutil.copy2(source / license_name, app / "Contents/Resources" / license_name)
        run(["codesign", "--force", "--deep", "--sign", "-", str(app)])
        run(["codesign", "--verify", "--deep", "--strict", str(app)])
        archive = root / "termust-macos-arm64.tar.gz"
        archive_app(app, archive)
        with tarfile.open(archive, "r:gz") as package:
            packed_info = plistlib.load(package.extractfile("Termust.app/Contents/Info.plist"))
        if (packed_info.get("CFBundleShortVersionString") != package_version
                or packed_info.get("CFBundleVersion") != str(build_timestamp)):
            raise RuntimeError("archived application version differs from the release metadata.")
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        installer = source / "install.sh"
        checksum = root / "archive.sha256"
        checksum.write_text(digest + "\n")
        name = f"macos/binary/{digest}.tar.gz"
        manifest = root / "latest-macos-arm64.json"
        manifest.write_text(json.dumps({
            "schema": 1, "platform": "macos-arm64", "archive": name, "sha256": digest,
            "package_version": package_version, "build_timestamp": build_timestamp,
        }, indent=2) + "\n")
        run(["b2", "file", "upload", "--no-progress", "--content-type", "application/gzip", "--cache-control", "public, max-age=31536000, immutable", BUCKET, str(archive), name])
        public_archive = root / "public.tar.gz"
        download(name, public_archive)
        if hashlib.sha256(public_archive.read_bytes()).hexdigest() != digest:
            raise RuntimeError("public binary archive checksum mismatch; installer was not changed.")
        test_home = root / "install-test-home"
        test_home.mkdir()
        run(["bash", str(installer), "--archive", str(public_archive), "--checksum-file", str(checksum)], env=dict(os.environ, HOME=str(test_home), PATH="/usr/bin:/bin:/usr/sbin:/sbin"))
        if not (test_home / "Applications/Termust.app/Contents/MacOS/termust").is_file():
            raise RuntimeError("binary installer verification failed; public installer was not changed.")
        run(["b2", "file", "upload", "--no-progress", "--content-type", "application/json", "--cache-control", "no-cache, no-store, max-age=0", BUCKET, str(manifest), "latest-macos-arm64.json"])
        public_manifest = root / "public-latest.json"
        download("latest-macos-arm64.json", public_manifest)
        if public_manifest.read_bytes() != manifest.read_bytes():
            raise RuntimeError("public macOS manifest differs from the uploaded manifest.")
        print(f"Published and verified macOS ARM64 archive {digest}.")
        print(f"Install with: curl -fsSL {BASE_URL}/install.sh | bash")


try:
    main()
except (RuntimeError, OSError, tarfile.TarError, subprocess.CalledProcessError) as error:
    print(f"Error: {error}", file=sys.stderr)
    sys.exit(1)
TERMUST_PYTHON
