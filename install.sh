#!/usr/bin/env bash

main() {
    if ! command -v python3 >/dev/null 2>&1; then
        printf '%s\n' 'Error: python3 is required. Install it with your package manager (apt install python3 / dnf install python3 / pacman -S python).' >&2
        return 1
    fi
    python3 - "$@" <<'TERMUST_PYTHON'
import argparse
import configparser
from contextlib import contextmanager
import ctypes
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import plistlib
import re
import shlex
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import time
from urllib.parse import urlsplit

BASE_URL = "https://f005.backblazeb2.com/file/termust"
RECEIPT = "share/doc/termust/install.json"
LINUX_ARCHIVE = "termust-linux-x86_64.tar.gz"
MAX_UPDATE_ARCHIVE = 128 * 1024 * 1024
LAST_UTC_TIMESTAMP = int(datetime.max.replace(microsecond=0, tzinfo=timezone.utc).timestamp())
FIXED_FILES = {
    "bin/termust", "share/applications/Termust.desktop",
    "share/icons/hicolor/scalable/apps/Termust.svg",
    "share/terminfo/alacritty.info", "share/bash-completion/completions/termust",
    "share/fish/vendor_completions.d/termust.fish", "share/zsh/site-functions/_termust",
}


def fail(message):
    raise RuntimeError(message)


def warning(message):
    print("Warning: " + message, file=sys.stderr)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def file_digest(path):
    checksum = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            checksum.update(chunk)
    return checksum.hexdigest()


def json_bytes(value):
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()


def version_id(value):
    return isinstance(value, str) and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}", value)


def managed_path(value):
    path = PurePosixPath(value)
    if path.is_absolute() or ".." in path.parts or str(path) != value:
        return False
    return value in FIXED_FILES or (value.startswith("share/doc/termust/")
        and len(path.parts) == 4 and not path.name.startswith("legacy-alacritty"))


def safe_destination(prefix, relative):
    if not managed_path(relative):
        fail("unsafe managed installation path: " + relative)
    path = prefix / relative
    for parent in path.parents:
        if parent == prefix:
            break
        if parent.is_symlink():
            fail("installation directory is a symbolic link: " + str(parent))
    if path.is_symlink() or (path.exists() and not path.is_file()):
        fail("installation destination is not a regular file: " + str(path))
    return path


def write_atomic(destination, data, mode=0o644):
    destination.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix=".termust-", dir=destination.parent)
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
        os.chmod(temporary, mode)
        os.replace(temporary, destination)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def distro_info():
    result = {}
    try:
        for line in Path("/etc/os-release").read_text().splitlines():
            if "=" in line:
                key, value = line.split("=", 1)
                fields = shlex.split(value)
                if fields:
                    result[key] = fields[0]
    except (OSError, ValueError):
        pass
    return result


def dependency_hint():
    distro = distro_info()
    families = set((distro.get("ID", "") + " " + distro.get("ID_LIKE", "")).split())
    if families & {"debian", "ubuntu"}:
        command = "sudo apt install curl python3 ca-certificates libfontconfig1 libfreetype6 libxkbcommon0 libxkbcommon-x11-0 libwayland-client0 libegl1 libgl1 libxcursor1 libxi6 libxrandr2 libx11-xcb1 ncurses-bin fonts-dejavu-core"
    elif families & {"fedora", "rhel", "centos"}:
        command = "sudo dnf install curl python3 ca-certificates fontconfig freetype libxkbcommon libxkbcommon-x11 libX11 libXcursor libXi libXrandr libxcb wayland mesa-libEGL mesa-libGL ncurses dejavu-sans-mono-fonts"
    elif families & {"arch", "manjaro"}:
        command = "sudo pacman -S --needed curl python fontconfig freetype2 libxkbcommon libxkbcommon-x11 libx11 libxcursor libxi libxrandr libxcb wayland libglvnd mesa ncurses ttf-dejavu"
    elif families & {"opensuse", "suse", "opensuse-tumbleweed", "opensuse-leap"}:
        command = "sudo zypper install curl python3 fontconfig libfreetype6 libxkbcommon0 libxkbcommon-x11-0 libwayland-client0 libEGL1 libGL1 libXcursor1 libXi6 libXrandr2 libX11-xcb1 ncurses-utils dejavu-fonts"
    else:
        command = "Install Python 3, curl, fontconfig, FreeType, xkbcommon, X11/Wayland and Mesa OpenGL using your distribution's package manager."
    return f"System: {distro.get('PRETTY_NAME', platform.system())}\nDependency command (not run automatically):\n  {command}"


def numeric_version(value):
    match = re.search(r"(?:^|\s)(\d+)\.(\d+)\.(\d+)", value)
    return tuple(map(int, match.groups())) if match else None


def executable_version(path):
    result = subprocess.run([str(path), "--version"], stdin=subprocess.DEVNULL,
        capture_output=True, text=True, timeout=10)
    if result.returncode:
        fail("this binary cannot run on this machine. The installed terminal was not changed.\n"
             + result.stdout + result.stderr + "\n" + dependency_hint())
    value = result.stdout.strip()
    if not value.startswith("termust "):
        fail("release executable is not Termust. The installed terminal was not changed.")
    return value


def runtime_check(candidate, metadata):
    with candidate.open("rb") as source:
        is_elf = source.read(4) == b"\x7fELF"
    if not is_elf and not metadata.get("minimum_glibc"):
        return
    try:
        libc = os.confstr("CS_GNU_LIBC_VERSION") or ""
    except (ValueError, OSError):
        libc = ""
    if not libc.startswith("glibc "):
        fail("this package needs glibc Linux; musl/Alpine is not supported. Build Termust from source on that system.")
    required = metadata.get("minimum_glibc")
    if required and tuple(map(int, libc.split()[1].split("."))) < tuple(map(int, required.split("."))):
        fail(f"this release needs glibc {required}; found {libc}. Use a compatible release or build from source. Do not replace the system libc.")
    if not is_elf:
        return
    missing = []
    for library in ("libfontconfig.so.1", "libfreetype.so.6", "libxkbcommon.so.0"):
        try:
            ctypes.CDLL(library)
        except OSError:
            missing.append(library)
    for alternatives in (("libEGL.so.1", "libGL.so.1"), ("libwayland-client.so.0", "libX11.so.6")):
        for library in alternatives:
            try:
                ctypes.CDLL(library)
                break
            except OSError:
                pass
        else:
            missing.append(" or ".join(alternatives))
    if missing:
        fail("missing runtime libraries: " + ", ".join(missing) + "\n" + dependency_hint())


def download(url, target, *, revalidate=False):
    if not shutil.which("curl"):
        fail("curl is required to download a release.\n" + dependency_hint())
    headers = ["--header", "Cache-Control: no-cache"] if revalidate else []
    subprocess.run(["curl", "--fail", "--show-error", "--silent", "--location",
        "--proto", "=https", "--proto-redir", "=https", "--tlsv1.2", "--retry", "3",
        "--connect-timeout", "15", "--max-time", "600", *headers, "--output", str(target), url], check=True)


@contextmanager
def install_lock(prefix, wait=0):
    descriptor = os.open(prefix / ".termust-install.lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "r+") as lock:
        info = os.fstat(lock.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_nlink != 1:
            fail("installation lock is not owned exclusively by the current user.")
        deadline = time.monotonic() + wait
        while True:
            try:
                fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                if time.monotonic() >= deadline:
                    fail("another Termust installation is running for this prefix.")
                time.sleep(0.1)
        yield


def update_download(relative, target, limit, timeout):
    command = ["curl", "-q", "--fail", "--silent", "--show-error", "--proto", "=https",
        "--tlsv1.2", "--max-redirs", "0", "--connect-timeout", "6", "--max-time", str(timeout),
        "--max-filesize", str(limit), "--header", "Cache-Control: no-cache",
        "--write-out", "\n%{http_code}", BASE_URL + "/" + relative]
    with target.open("w+b") as output, subprocess.Popen(command, stdout=subprocess.PIPE) as process:
        try:
            while True:
                chunk = process.stdout.read(65536)
                if not chunk:
                    break
                if output.tell() + len(chunk) > limit + 4:
                    fail("update download exceeds its size limit.")
                output.write(chunk)
            if process.wait() != 0 or output.tell() < 4:
                fail("could not download the update from the Termust release server.")
            size = output.tell() - 4
            output.seek(size)
            if output.read() != b"\n200":
                fail("unexpected HTTP response from the Termust release server.")
            output.truncate(size)
        finally:
            if process.poll() is None:
                process.kill()


def unpack(archive, checksum, target, root_name="termust"):
    if not archive.is_file() or not checksum.is_file():
        fail("the release archive or SHA256 file is missing.")
    match = re.fullmatch(r"([0-9a-fA-F]{64})(?:[ \t]+[^\r\n]+)?", checksum.read_text(encoding="ascii").strip())
    if not match:
        fail("invalid SHA256 file.")
    actual = file_digest(archive)
    if actual != match[1].lower():
        fail("SHA256 checksum mismatch. The installed terminal was not changed.")
    with tarfile.open(archive, "r:gz") as package:
        members = package.getmembers()
        if len(members) > 10000 or sum(member.size for member in members) > 1024 ** 3:
            fail("release archive is too large.")
        seen = set()
        for member in members:
            path = PurePosixPath(member.name)
            if (not member.name or path.is_absolute() or ".." in path.parts
                    or not path.parts or path.parts[0] != root_name or str(path) in seen
                    or any(ord(c) < 32 or ord(c) == 127 for c in member.name)
                    or not (member.isdir() or member.isfile())):
                fail("unsafe release archive entry.")
            seen.add(str(path))
        for member in members:
            destination = target.joinpath(*PurePosixPath(member.name).parts)
            if member.isdir():
                destination.mkdir(parents=True, exist_ok=True)
            else:
                destination.parent.mkdir(parents=True, exist_ok=True)
                with package.extractfile(member) as source, destination.open("xb") as output:
                    shutil.copyfileobj(source, output)
                destination.chmod(0o755 if member.mode & 0o111 else 0o644)
    return actual


def read_metadata(package, version, binary):
    metadata_path = package / "release.json"
    metadata = json.loads(metadata_path.read_text()) if metadata_path.is_file() else {}
    if not isinstance(metadata, dict):
        fail("invalid Linux release metadata.")
    if metadata and (metadata.get("version") != version or metadata.get("binary_sha256") != file_digest(binary)):
        fail("release metadata does not match VERSION or executable checksum.")
    if metadata.get("platform", "linux-x86_64") != "linux-x86_64":
        fail("release metadata has an unsupported platform.")
    minimum = metadata.get("minimum_glibc")
    if minimum is not None and (not isinstance(minimum, str) or not re.fullmatch(r"\d+\.\d+", minimum)):
        fail("invalid minimum_glibc in release metadata.")
    timestamp = metadata.get("build_timestamp", 0)
    if not isinstance(timestamp, int) or isinstance(timestamp, bool) or timestamp < 0:
        fail("invalid build_timestamp in release metadata.")
    return metadata


def installed_receipt(prefix, *, reject_modified=True):
    path = prefix / RECEIPT
    if not path.exists():
        return {}
    safe_destination(prefix, RECEIPT)
    value = json.loads(path.read_text())
    if not isinstance(value, dict):
        fail("invalid Termust installation receipt.")
    binary = prefix / "bin/termust"
    if not binary.is_file() or value.get("binary_sha256") != file_digest(binary):
        if reject_modified:
            fail("installed executable does not match its receipt; inspect the installation before replacing it.")
        return {}
    return value


def check_downgrade(prefix, candidate_version, metadata, allow):
    binary = prefix / "bin/termust"
    if not binary.is_file():
        return {}
    previous = installed_receipt(prefix)
    old_version = previous.get("binary_version", "")
    if not old_version:
        try:
            old_version = executable_version(binary)
        except (RuntimeError, OSError, subprocess.SubprocessError):
            warning("the existing executable has no verified Termust version; it will be backed up before replacement.")
    current = numeric_version(old_version)
    candidate = numeric_version(candidate_version)
    older = current and candidate and candidate < current
    old_time, new_time = previous.get("build_timestamp", 0), metadata.get("build_timestamp", 0)
    if current == candidate and old_time and new_time and new_time < old_time:
        older = True
    if current == candidate and old_time and not new_time and not allow:
        fail("this release has no build timestamp to compare with the installed version. Use --allow-downgrade to install it explicitly.")
    if older and not allow:
        fail("the installed version is newer than this release. Use --allow-downgrade to replace it explicitly.")
    return previous


def snapshot(prefix, names, target):
    target.mkdir(parents=True)
    entries = {}
    for relative in sorted(names):
        path = safe_destination(prefix, relative)
        if not path.exists():
            entries[relative] = None
            continue
        if not owned_file(path):
            fail("cannot back up an installation file not owned exclusively by this user: " + str(path))
        data = path.read_bytes()
        payload = str(len(entries))
        mode = stat.S_IMODE(path.stat().st_mode) & 0o777
        write_atomic(target / payload, data, mode)
        entries[relative] = {"payload": payload, "sha256": digest(data), "mode": mode}
    write_atomic(target / "manifest.json", json_bytes({"schema_version": 1, "files": entries}))


def load_snapshot(prefix, target):
    if target.is_symlink() or not owned_file(target / "manifest.json"):
        fail("no trusted rollback snapshot is available.")
    manifest = json.loads((target / "manifest.json").read_text())
    entries = manifest.get("files", {})
    if manifest.get("schema_version") != 1 or not isinstance(entries, dict) or not 1 <= len(entries) <= 10000 or not entries.get("bin/termust"):
        fail("invalid rollback snapshot.")
    result = {}
    for relative, entry in entries.items():
        safe_destination(prefix, relative)
        if entry is None:
            result[relative] = None
            continue
        payload = entry.get("payload", "")
        if not isinstance(payload, str) or not re.fullmatch(r"[0-9]+", payload):
            fail("unsafe rollback payload.")
        path = target / payload
        if not owned_file(path) or digest(path.read_bytes()) != entry.get("sha256"):
            fail("rollback backup checksum mismatch; the installed terminal was not changed.")
        if not isinstance(entry.get("mode"), int) or not 0 <= entry["mode"] <= 0o777:
            fail("invalid rollback file permissions.")
        result[relative] = (path.read_bytes(), entry["mode"])
    return result


def save_previous(prefix, names):
    parent = prefix / "share/termust"
    if any(path.is_symlink() for path in (prefix / "share", parent, parent / "rollback")):
        fail("rollback storage must not be a symbolic link.")
    parent.mkdir(parents=True, exist_ok=True)
    scratch = tempfile.TemporaryDirectory(prefix=".rollback-", dir=parent)
    try:
        snapshot(prefix, names, Path(scratch.name) / "snapshot")
    except BaseException:
        scratch.cleanup()
        raise
    return scratch


def commit_previous(prefix, scratch):
    destination = prefix / "share/termust/rollback"
    previous = Path(scratch.name) / "previous"
    if destination.exists():
        os.replace(destination, previous)
    try:
        os.replace(Path(scratch.name) / "snapshot", destination)
    except OSError:
        if previous.exists():
            os.replace(previous, destination)
        raise
    print("Previous installation saved for --rollback.")


def restore(prefix, entries):
    for relative in sorted(entries, key=lambda value: value == "bin/termust"):
        destination = safe_destination(prefix, relative)
        value = entries[relative]
        if value is None:
            if destination.exists():
                destination.unlink()
        else:
            if not destination.is_file() or destination.read_bytes() != value[0] or stat.S_IMODE(destination.stat().st_mode) != value[1]:
                write_atomic(destination, *value)


def rollback(prefix):
    entries = load_snapshot(prefix, prefix / "share/termust/rollback")
    with tempfile.TemporaryDirectory(prefix="termust-rollback-") as temporary:
        candidate = Path(temporary) / "termust"
        write_atomic(candidate, *entries["bin/termust"])
        version = executable_version(candidate)
        current = Path(temporary) / "current"
        snapshot(prefix, entries, current)
        try:
            restore(prefix, entries)
        except (OSError, RuntimeError):
            restore(prefix, load_snapshot(prefix, current))
            raise
    refresh_integrations(prefix, (prefix / "share/applications/Termust.desktop").is_file())
    print("Restored previous installation: " + version)
    diagnose(prefix, False)


def refresh_integrations(prefix, desktop):
    source = prefix / "share/terminfo/alacritty.info"
    if shutil.which("tic") and source.is_file():
        for destination in (prefix / "share/terminfo", Path.home() / ".terminfo"):
            result = subprocess.run(["tic", "-x", "-e", "alacritty,alacritty-direct", "-o", str(destination), str(source)], capture_output=True, text=True)
            if result.returncode:
                warning(f"terminfo could not be updated in {destination}.\n{result.stderr.strip()}")
    elif not shutil.which("tic"):
        warning("tic is unavailable; install ncurses tools for terminal capability entries.")
    if desktop and shutil.which("update-desktop-database"):
        result = subprocess.run(["update-desktop-database", str(prefix / "share/applications")], capture_output=True, text=True)
        if result.returncode:
            warning("the desktop database could not be updated: " + result.stderr.strip())


def previous_process(binary, installed_hash):
    for process in Path("/proc").glob("[0-9]*"):
        executable = process / "exe"
        try:
            if process.stat().st_uid != os.getuid() or os.readlink(executable) != f"{binary} (deleted)":
                continue
            if digest(executable.read_bytes()) != installed_hash:
                return process.name
        except OSError:
            continue
    return None


def diagnose(prefix, strict):
    binary = prefix / "bin/termust"
    good = binary.is_file()
    if not good:
        warning("Termust is not installed at " + str(binary))
    else:
        print("Executable: " + str(binary))
        receipt = installed_receipt(prefix)
        if receipt:
            print("Installed package: " + receipt["version"])
            pid = previous_process(binary, receipt["binary_sha256"])
            if pid:
                print(f"Termust PID {pid} is still running a previous build. Its windows and tabs keep that build.")
        print("Launch the installed build in a separate window: " + shlex.quote(str(binary)))
    selected = shutil.which("termust")
    if not selected or Path(selected).resolve() != binary.resolve():
        warning("shell PATH selects " + (selected or "no termust executable") + " instead of " + str(binary))
        print("Put " + str(prefix / "bin") + " first in PATH. Then open a new shell or run hash -r (Bash) / rehash (Zsh).")
        good = False
    else:
        print("Shell PATH selects the installed Termust.")
    copies = set()
    for directory in os.get_exec_path():
        path = Path(directory) / "termust"
        if path.is_file() and os.access(path, os.X_OK) and path.resolve() != binary.resolve():
            copies.add(str(path))
    for path in sorted(copies):
        warning("another Termust copy remains at " + path)
    launcher = prefix / "share/applications/Termust.desktop"
    if launcher.is_file():
        config = configparser.ConfigParser(interpolation=None)
        config.optionxform = str
        config.read(launcher)
        entry = config["Desktop Entry"]
        expected = desktop_command(str(binary))
        if entry.get("Exec") != expected or entry.get("TryExec") != str(binary).replace("\\", "\\\\"):
            warning("desktop launcher does not select the installed executable: " + str(launcher))
            good = False
        else:
            print("Desktop launcher: " + str(launcher))
        roots = [Path(os.environ.get("XDG_DATA_HOME") or str(Path.home() / ".local/share"))]
        roots += [Path(p) for p in (os.environ.get("XDG_DATA_DIRS") or "/usr/local/share:/usr/share").split(":") if p]
        visible = next((root / "applications/Termust.desktop" for root in roots if (root / "applications/Termust.desktop").is_file()), None)
        if visible is None or visible.resolve() != launcher.resolve():
            warning("desktop search path selects " + (str(visible) if visible else "no Termust launcher") + "; add " + str(prefix / "share") + " to XDG_DATA_DIRS for this custom prefix.")
            if visible is not None:
                good = False
    elif good and receipt.get("desktop", False):
        warning("the managed desktop launcher is missing; run the installer again to repair it.")
        good = False
    if strict and not good:
        fail("installation check found a launch-path problem.")


def install(package, prefix, version, metadata, archive_hash, desktop, allow):
    binary = package / "bin/termust"
    runtime_check(binary, metadata)
    binary_version = executable_version(binary)
    if metadata.get("binary_version", binary_version) != binary_version:
        fail("release metadata does not match the executable version.")
    previous = check_downgrade(prefix, binary_version, metadata, allow)
    files = {}
    for relative in FIXED_FILES:
        source = package / relative
        if not desktop and relative.startswith(("share/applications/", "share/icons/")):
            continue
        if source.is_file():
            files[relative] = (source.read_bytes(), 0o755 if relative == "bin/termust" else 0o644)
    documentation = package / "share/doc/termust"
    if documentation.is_dir():
        for source in documentation.iterdir():
            relative = str(source.relative_to(package))
            if source.is_file() and managed_path(relative) and relative != RECEIPT:
                files[relative] = (source.read_bytes(), 0o644)
    receipt = dict(metadata)
    receipt.update(version=version, binary_version=binary_version, binary_sha256=digest(binary.read_bytes()),
        archive_sha256=archive_hash, desktop=desktop or previous.get("desktop", False), schema_version=1)
    files["share/doc/termust/VERSION"] = ((version + "\n").encode(), 0o644)
    files[RECEIPT] = (json_bytes(receipt), 0o644)
    if desktop:
        launcher = (package / "share/applications/Termust.desktop").read_text()
        executable = str(prefix / "bin/termust")
        section, lines = "", []
        for line in launcher.splitlines():
            if line.startswith("["):
                section = line
            if section == "[Desktop Entry]" and line.startswith("Name="):
                line = "Name=Termust"
            elif line.startswith("TryExec="):
                line = "TryExec=" + executable.replace("\\", "\\\\")
            elif line.startswith("Exec="):
                line = "Exec=" + desktop_command(executable)
            lines.append(line)
        launcher = "\n".join(lines) + "\n"
        current_launcher = prefix / "share/applications/Termust.desktop"
        if current_launcher.is_file():
            launcher = preserve_personal_icon(current_launcher.read_text(), launcher)
        files["share/applications/Termust.desktop"] = (launcher.encode(), 0o644)
    destination = safe_destination(prefix, "bin/termust")
    identical = destination.is_file() and digest(destination.read_bytes()) == receipt["binary_sha256"] and previous.get("version") == version
    pending_backup = None
    if destination.is_file() and not identical:
        names = set(files) | {name for name in FIXED_FILES if (prefix / name).is_file()}
        documentation = prefix / "share/doc/termust"
        if documentation.is_dir():
            names |= {str(p.relative_to(prefix)) for p in documentation.iterdir() if p.is_file() and managed_path(str(p.relative_to(prefix)))}
        pending_backup = save_previous(prefix, names)
    incomplete = False
    core_names = ("bin/termust", "share/doc/termust/VERSION", RECEIPT)
    previous_core = {}
    existing = destination.is_file()
    try:
        for relative in core_names:
            path = safe_destination(prefix, relative)
            previous_core[relative] = (path.read_bytes(), stat.S_IMODE(path.stat().st_mode)) if path.is_file() else None
        for relative in core_names:
            if previous_core[relative] != files[relative]:
                write_atomic(safe_destination(prefix, relative), *files[relative])
        if pending_backup:
            commit_previous(prefix, pending_backup)
    except (OSError, RuntimeError) as error:
        if existing:
            restore(prefix, previous_core)
            fail("update could not save its receipt; the previous installation was restored: " + str(error))
        if not destination.is_file():
            raise
        incomplete = True
        warning("some optional integration files could not be installed: " + str(error))
    finally:
        if pending_backup:
            pending_backup.cleanup()
    for relative in core_names:
        files.pop(relative)
    desktop_ready = all(name in files for name in ("share/applications/Termust.desktop", "share/icons/hicolor/scalable/apps/Termust.svg"))
    for relative, content in files.items():
        try:
            write_atomic(safe_destination(prefix, relative), *content)
        except (OSError, RuntimeError) as error:
            incomplete = True
            if relative.startswith(("share/applications/", "share/icons/")):
                desktop_ready = False
            warning("some optional integration files could not be installed: " + str(error))
    if desktop and desktop_ready:
        try:
            migrate_legacy_install(prefix)
        except OSError as error:
            warning("previous fork integration could not be migrated: " + str(error))
    refresh_integrations(prefix, desktop)
    if identical:
        print(f"Termust {version} is already installed.")
    elif previous.get("version"):
        print(f"Updated Termust from {previous['version']} to {version}.")
    else:
        print(f"Installed Termust {version}.")
    print(binary_version)
    if incomplete:
        warning("binary installed, but integration is incomplete. Fix the reported paths and run the installer again.")
    diagnose(prefix, False)


def macos_validate_app(app):
    binary = app / "Contents/MacOS/termust"
    info = app / "Contents/Info.plist"
    if not app.is_dir() or not binary.is_file() or not os.access(binary, os.X_OK) or not info.is_file():
        fail(f"unrecognized or incomplete application at {app}; no existing application was replaced.")
    bundle = plistlib.loads(info.read_bytes())
    if not isinstance(bundle, dict) or bundle.get("CFBundleIdentifier") != "com.tarcisiomiranda.termust":
        fail(f"unrecognized application at {app}; no existing application was replaced.")
    subprocess.run(["codesign", "--verify", "--deep", "--strict", str(app)], check=True)
    return bundle


def macos_app_hash(app):
    result = hashlib.sha256()
    for path in sorted(app.rglob("*")):
        if path.is_symlink() or not (path.is_file() or path.is_dir()):
            fail(f"unsupported application entry at {path}.")
        if path.is_file():
            result.update(path.relative_to(app).as_posix().encode() + b"\0")
            result.update(str(path.stat().st_mode & 0o111).encode() + b"\0")
            result.update(bytes.fromhex(digest(path.read_bytes())))
    return result.hexdigest()


def macos_activate(app, directory, archive_hash, root, previous):
    destination = directory / "Termust.app"
    managed = directory / ".termust"
    releases = managed / "releases"
    for path in (managed, releases, releases / archive_hash):
        if path.is_symlink() or (path.exists() and (not path.is_dir() or path.stat().st_uid != os.getuid())):
            fail(f"unsafe managed application directory: {path}.")
        path.mkdir(exist_ok=True)
    release = releases / archive_hash / "Termust.app"
    if release.exists() or release.is_symlink():
        if release.is_symlink() or macos_app_hash(release) != macos_app_hash(app):
            fail("the existing versioned bundle differs from the verified package.")
    else:
        os.rename(app, release)
    if previous == release:
        print(f"Termust package {archive_hash[:12]} is already installed at {destination}.")
        return
    if destination.exists() or destination.is_symlink():
        if previous is None or destination.resolve() != previous:
            fail("the installed application changed during installation; it was not replaced.")
    elif previous is not None:
        fail("the installed application disappeared during installation; it was not replaced.")
    moved = False
    if previous is not None and not destination.is_symlink():
        backup_parent = releases / ("previous-" + macos_app_hash(destination))
        if backup_parent.exists() or backup_parent.is_symlink():
            fail(f"previous bundle backup already exists at {backup_parent}; no existing application was replaced.")
        backup_parent.mkdir()
        previous = backup_parent / "Termust.app"
        os.rename(destination, previous)
        moved = True
    launcher = root / "launcher"
    try:
        os.symlink(os.path.relpath(release, directory), launcher)
        os.replace(launcher, destination)
    except OSError:
        if moved:
            os.rename(previous, destination)
            previous.parent.rmdir()
        raise
    if previous is not None:
        previous_link = root / "previous-launcher"
        os.symlink(os.path.relpath(previous, managed), previous_link)
        try:
            os.replace(previous_link, managed / "previous.app")
        except OSError as error:
            warning(f"could not create the previous-version shortcut: {error}")
        print(f"Previous application preserved at {previous}")
        print(f"Updated Termust at {destination} to package {archive_hash[:12]}.")
    else:
        print(f"Installed Termust at {destination} (package {archive_hash[:12]}).")


def install_macos_archive(archive, checksum, root, directory, previous, manifest=None):
    archive_hash = unpack(archive, checksum, root, "Termust.app")
    app = root / "Termust.app"
    bundle = macos_validate_app(app)
    if manifest is not None and (bundle.get("CFBundleShortVersionString") != manifest["package_version"]
            or bundle.get("CFBundleVersion") != str(manifest["build_timestamp"])):
        fail("the macOS package version differs from the offered update.")
    binary_version = executable_version(app / "Contents/MacOS/termust")
    if manifest is not None and binary_version.split()[1:2] != [manifest["package_version"]]:
        fail("the macOS package version differs from the offered update.")
    print(binary_version)
    macos_activate(app, directory, archive_hash=archive_hash, root=root, previous=previous)


def install_linux_archive(archive, checksum, root, prefix, version, desktop, allow, manifest=None):
    archive_hash = unpack(archive, checksum, root)
    package = root / "termust"
    binary = package / "bin/termust"
    if not binary.is_file() or not os.access(binary, os.X_OK) or not (package / "VERSION").is_file():
        fail("release is missing its executable or VERSION file.")
    package_version = (package / "VERSION").read_text().strip()
    if not version_id(package_version) or (version and version != package_version):
        fail("release VERSION does not match the requested package.")
    metadata = read_metadata(package, package_version, binary)
    fields = ("platform", "version", "package_version", "binary_version", "binary_sha256", "build_timestamp")
    if manifest is not None and any(metadata.get(key) != manifest[key] for key in fields):
        fail("the Linux package metadata differs from the offered update.")
    install(package, prefix, package_version, metadata, archive_hash, desktop, allow)


def macos_main():
    parser = argparse.ArgumentParser(description="Install the native Termust app for macOS Apple Silicon without sudo or Rust.")
    parser.add_argument("--prefix", default=str(Path.home() / "Applications"), help="directory for Termust.app")
    parser.add_argument("--base-url", default=BASE_URL)
    parser.add_argument("--archive", type=Path)
    parser.add_argument("--checksum-file", type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    if platform.machine() != "arm64":
        fail(f"unsupported platform: Darwin {platform.machine()}; macOS requires Apple Silicon arm64.")
    if any(ord(c) < 32 or ord(c) == 127 for c in args.prefix):
        fail("the installation prefix contains a control character.")
    directory = Path(args.prefix).expanduser().resolve()
    destination = directory / "Termust.app"
    if args.check:
        if args.archive or args.checksum_file:
            fail("--check cannot be combined with archive options.")
        macos_validate_app(destination)
        print(executable_version(destination / "Contents/MacOS/termust"))
        return
    if args.checksum_file and not args.archive:
        fail("--checksum-file needs --archive.")
    directory.mkdir(parents=True, exist_ok=True)
    with install_lock(directory):
        previous = None
        if destination.exists() or destination.is_symlink():
            previous = destination.resolve()
            releases = directory / ".termust/releases"
            if destination.is_symlink() and (previous.parent.parent != releases or previous.name != "Termust.app"):
                fail("the application link points outside the managed releases; it was not changed.")
            if not previous.exists() or previous.stat().st_uid != os.getuid():
                fail("the existing application is missing or is owned by another user; it was not changed.")
            macos_validate_app(destination)
        with tempfile.TemporaryDirectory(prefix=".termust-install-", dir=directory) as temporary:
            root = Path(temporary)
            archive = args.archive
            checksum = args.checksum_file
            if archive is None:
                parsed = urlsplit(args.base_url)
                if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password or parsed.query or parsed.fragment or any(ord(c) <= 32 or ord(c) == 127 for c in args.base_url):
                    fail("--base-url must be a public HTTPS URL without a query or credentials.")
                base = args.base_url.rstrip("/")
                manifest_path = root / "latest.json"
                download(base + "/latest-macos-arm64.json", manifest_path, revalidate=True)
                manifest = json.loads(manifest_path.read_text())
                sha = manifest.get("sha256", "") if isinstance(manifest, dict) else ""
                if (not isinstance(manifest, dict) or manifest.get("schema") != 1 or manifest.get("platform") != "macos-arm64"
                        or not isinstance(sha, str) or not re.fullmatch(r"[0-9a-f]{64}", sha)
                        or manifest.get("archive") != f"macos/binary/{sha}.tar.gz"):
                    fail("invalid macOS release manifest.")
                print(f"Latest macOS arm64 package: {sha[:12]}", flush=True)
                archive = root / "application.tar.gz"
                checksum = root / "application.sha256"
                download(base + "/" + manifest["archive"], archive)
                checksum.write_text(sha + "\n")
            checksum = checksum or Path(str(archive) + ".sha256")
            install_macos_archive(archive, checksum, root, directory, previous)
    print("Launch the installed build in a separate window: open -n " + shlex.quote(str(destination)))


def sha256_id(value):
    return isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) is not None


def release_order(version, timestamp):
    if (not isinstance(version, str) or len(version) > 80 or type(timestamp) is not int
            or not 0 < timestamp <= LAST_UTC_TIMESTAMP):
        return None
    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+[0-9A-Za-z.-]+)?", version, re.ASCII)
    if not match:
        return None
    prerelease = match[4]
    components = tuple((0, int(item)) if item.isdecimal() else (1, item)
        for item in prerelease.split(".")) if prerelease else ()
    return tuple(map(int, match.groups()[:3])), prerelease is None, components, timestamp


def managed_update(prefix, target):
    if (not target or not prefix.is_dir() or prefix.resolve() != prefix
            or prefix.stat().st_uid != os.getuid()):
        return None
    if target == "linux-x86_64":
        binary, receipt = prefix / "bin/termust", prefix / RECEIPT
        if any(path.resolve() != path or not owned_file(path) for path in (binary, receipt)):
            return None
        data = installed_receipt(prefix, reject_modified=False)
        order = release_order(data.get("package_version"), data.get("build_timestamp"))
        if (type(data.get("schema_version")) is not int or data["schema_version"] != 1 or data.get("platform") != target
                or not order or type(data.get("desktop")) is not bool
                or not version_id(data.get("version")) or not sha256_id(data.get("archive_sha256"))
                or not isinstance(data.get("binary_version"), str)
                or data["binary_version"].split()[:2] != ["termust", data["package_version"]]):
            return None
        return {"id": data["version"], "sha256": data["archive_sha256"], "order": order, "desktop": data["desktop"]}
    launcher = prefix / "Termust.app"
    if not launcher.is_symlink() or launcher.lstat().st_uid != os.getuid():
        return None
    app = launcher.resolve()
    release_id = app.parent.name
    expected = prefix / ".termust/releases" / release_id / "Termust.app"
    if (not sha256_id(release_id) or app != expected or prefix / os.readlink(launcher) != expected
            or any(not path.is_dir() or path.stat().st_uid != os.getuid()
                for path in (app, app.parent, app.parent.parent, app.parent.parent.parent))):
        return None
    bundle = macos_validate_app(app)
    build = bundle.get("CFBundleVersion")
    order = release_order(bundle.get("CFBundleShortVersionString"), int(build)) if (
        isinstance(build, str) and re.fullmatch(r"[0-9]{1,19}", build)) else None
    if order:
        return {"id": release_id, "sha256": release_id, "order": order, "app": app}
    return None


def latest_update(root, target):
    metadata_file = root / "manifest.json"
    if target == "linux-x86_64":
        pointer = root / "latest.txt"
        update_download("latest-linux-x86_64.txt", pointer, 256, 15)
        release_id = pointer.read_text(encoding="ascii").strip()
        if not version_id(release_id):
            fail("invalid Linux update pointer.")
        update_download(f"releases/{release_id}/release.json", metadata_file, 65536, 15)
    else:
        update_download("latest-macos-arm64.json", metadata_file, 65536, 15)
    data = json.loads(metadata_file.read_bytes())
    if not isinstance(data, dict) or data.get("platform") != target or not sha256_id(data.get("sha256")):
        fail("invalid Termust update manifest.")
    sha = data["sha256"]
    version = data.get("package_version")
    order = release_order(version, data.get("build_timestamp"))
    if target == "linux-x86_64":
        if (not order or data.get("version") != release_id or data.get("archive") != LINUX_ARCHIVE
                or not sha256_id(data.get("binary_sha256")) or not isinstance(data.get("binary_version"), str)
                or data["binary_version"].split()[:2] != ["termust", version]
                or type(data.get("size")) is not int or not 0 < data["size"] <= MAX_UPDATE_ARCHIVE):
            fail("invalid Linux update manifest.")
        archive = f"releases/{release_id}/{LINUX_ARCHIVE}"
    else:
        release_id = sha
        archive = f"macos/binary/{sha}.tar.gz"
        if (type(data.get("schema")) is not int or data["schema"] != 1 or data.get("archive") != archive
                or (not order and ("package_version" in data or "build_timestamp" in data))):
            fail("invalid macOS update manifest.")
    label = ""
    if order:
        built = datetime.fromtimestamp(data["build_timestamp"], timezone.utc)
        label = f"{version} ({built:%Y-%m-%d %H:%M:%S} UTC)"
    return {"id": release_id, "sha256": sha, "version": label, "order": order, "archive": archive, "manifest": data}


def check_update(prefix, target, local, root, requested=None):
    if requested == (local["id"], local["sha256"]):
        return {"status": "current"}
    offer = latest_update(root, target)
    if requested is not None and requested != (offer["id"], offer["sha256"]):
        fail("the offered release changed; check for updates again.")
    if local["id"] == offer["id"] and local["sha256"] != offer["sha256"]:
        fail("the release checksum changed for an installed version.")
    newer = (offer["order"] and offer["order"] > local["order"] and local["id"] != offer["id"]
        and (offer["order"][0] != local["order"][0] or offer["order"][-1] >= local["order"][-1]))
    if not newer:
        return {"status": "current"}
    if requested is None:
        return {"status": "available", **{key: offer[key] for key in ("version", "id", "sha256")}}
    archive, checksum = root / "package.tar.gz", root / "package.sha256"
    manifest = offer["manifest"]
    limit = manifest["size"] if target == "linux-x86_64" else MAX_UPDATE_ARCHIVE
    update_download(offer["archive"], archive, limit, 180)
    if target == "linux-x86_64" and archive.stat().st_size != limit:
        fail("the Linux update archive has an unexpected size.")
    checksum.write_text(offer["sha256"] + "\n", encoding="ascii")
    if target == "linux-x86_64":
        install_linux_archive(archive, checksum, root, prefix, offer["id"], local["desktop"], False, manifest)
    else:
        install_macos_archive(archive, checksum, root, prefix, local["app"], manifest)
    return {"status": "installed"}


def update_main():
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--prefix", required=True, type=Path)
    modes = parser.add_mutually_exclusive_group(required=True)
    modes.add_argument("--update-status", action="store_true")
    modes.add_argument("--apply-update", metavar="ID")
    parser.add_argument("--expected-sha256")
    args = parser.parse_args()
    prefix = args.prefix
    if not prefix.is_absolute() or any(ord(c) < 32 or ord(c) == 127 for c in str(prefix)):
        fail("the updater requires an absolute installation prefix.")
    if args.update_status and args.expected_sha256 is not None:
        fail("--expected-sha256 requires --apply-update.")
    if args.apply_update is not None and (not version_id(args.apply_update) or not sha256_id(args.expected_sha256)):
        fail("the updater requires an exact release ID and SHA256.")
    platforms = {("Linux", "x86_64"): "linux-x86_64", ("Darwin", "arm64"): "macos-arm64"}
    target = platforms.get((platform.system(), platform.machine()))
    if args.update_status:
        local = managed_update(prefix, target)
        if not local:
            print(json.dumps({"status": "unmanaged"}))
            return
        with tempfile.TemporaryDirectory(prefix="termust-update-check-") as temporary:
            result = check_update(prefix, target, local, Path(temporary))
    else:
        lock = prefix / ".termust-install.lock"
        if (lock.resolve() != lock or not owned_file(lock)) and not managed_update(prefix, target):
            fail("this installation is not managed by the Termust installer.")
        with install_lock(prefix, wait=240):
            local = managed_update(prefix, target)
            if not local:
                fail("this installation is not managed by the Termust installer.")
            with tempfile.TemporaryDirectory(prefix=".termust-update-", dir=prefix) as temporary:
                result = check_update(prefix, target, local, Path(temporary), (args.apply_update, args.expected_sha256))
    print(json.dumps(result))


def main():
    if any(arg.split("=", 1)[0] in ("--update-status", "--apply-update") for arg in sys.argv[1:]):
        update_main()
        return
    if platform.system() == "Darwin":
        macos_main()
        return
    parser = argparse.ArgumentParser(description="Install Termust for Linux x86_64 or macOS Apple Silicon without sudo. Needs Bash, python3 and curl for downloads.")
    parser.add_argument("--prefix", default=str(Path.home() / ".local"))
    parser.add_argument("--version", default="")
    parser.add_argument("--base-url", default=BASE_URL)
    parser.add_argument("--archive", type=Path)
    parser.add_argument("--checksum-file", type=Path)
    parser.add_argument("--no-desktop", action="store_true")
    parser.add_argument("--allow-downgrade", action="store_true", help="explicitly replace a newer installed version")
    actions = parser.add_mutually_exclusive_group()
    actions.add_argument("--rollback", action="store_true", help="restore the verified previous installation without downloading")
    actions.add_argument("--check", action="store_true", help="check installation and launch paths without changing files")
    args = parser.parse_args()
    if platform.system() != "Linux" or platform.machine() != "x86_64":
        fail(f"unsupported platform: {platform.system()} {platform.machine()}; supported targets are Linux x86_64 and macOS arm64.")
    if any(ord(c) < 32 or ord(c) == 127 for c in args.prefix):
        fail("the installation prefix contains a control character.")
    prefix = Path(args.prefix).expanduser().resolve()
    if args.version and not version_id(args.version):
        fail("invalid release version.")
    if args.check or args.rollback:
        if args.archive or args.version or args.checksum_file or args.allow_downgrade or args.no_desktop:
            fail("--check and --rollback cannot be combined with installation options.")
    if args.check:
        diagnose(prefix, True)
        return
    if args.checksum_file and not args.archive:
        fail("--checksum-file needs --archive.")
    if args.rollback and not prefix.is_dir():
        fail("no previous installation is available for rollback.")
    prefix.mkdir(parents=True, exist_ok=True)
    with install_lock(prefix):
        if args.rollback:
            rollback(prefix)
            return
        with tempfile.TemporaryDirectory(prefix="termust-install-") as temporary:
            target = Path(temporary)
            version = args.version
            archive = args.archive
            checksum = args.checksum_file
            if archive is None:
                parsed = urlsplit(args.base_url)
                if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password or parsed.query or parsed.fragment or any(ord(c) <= 32 or ord(c) == 127 for c in args.base_url):
                    fail("--base-url must be a public HTTPS URL without a query or credentials.")
                base = args.base_url.rstrip("/")
                if not version:
                    download(base + "/latest-linux-x86_64.txt", target / "latest", revalidate=True)
                    version = (target / "latest").read_text().strip()
                    if not version_id(version):
                        fail("the latest release pointer is invalid.")
                    print(f"Latest Linux x86_64 package: {version}", flush=True)
                if "-workspace." in version or "-workspace-" in version:
                    fail(f"pre-Termust releases use their original installer: {base}/releases/{version}/install.sh")
                archive = target / "termust-linux-x86_64.tar.gz"
                checksum = target / (archive.name + ".sha256")
                download(f"{base}/releases/{version}/{archive.name}", archive)
                download(f"{base}/releases/{version}/{checksum.name}", checksum)
            checksum = checksum or Path(str(archive) + ".sha256")
            install_linux_archive(archive, checksum, target, prefix, version, not args.no_desktop, args.allow_downgrade)


def desktop_command(executable):
    escaped = executable.replace("\\", "\\\\").replace('"', '\\"').replace("`", "\\`").replace("$", "\\$")
    return ('"' + escaped.replace("%", "%%") + '"').replace("\\", "\\\\")


def preserve_personal_icon(current, replacement):
    values = {}
    section = ""
    for line in current.splitlines():
        if line.startswith("["):
            section = line
        elif section == "[Desktop Entry]" and "=" in line:
            key, value = line.split("=", 1)
            if key in ("Icon", "X-Termust-IconPreset", "X-Termust-OriginalIcon"):
                values[key] = value
    if values.get("X-Termust-IconPreset") not in ("cat", "dog", "fox", "robot", "yorkie"):
        return replacement
    icon = values.get("Icon", "").replace("\\\\", "\\")
    if not Path(icon).is_absolute() or not Path(icon).is_file():
        return replacement
    lines = []
    section = ""
    for line in replacement.splitlines():
        if line.startswith("["):
            section = line
        if line == "[Desktop Entry]":
            lines.extend([line, "Icon=" + values["Icon"],
                "X-Termust-IconPreset=" + values["X-Termust-IconPreset"],
                "X-Termust-OriginalIcon=" + values.get("X-Termust-OriginalIcon", "Termust")])
        elif section != "[Desktop Entry]" or not line.startswith("Icon="):
            lines.append(line)
    return "\n".join(lines) + "\n"


def owned_file(path):
    try:
        metadata = path.lstat()
    except OSError:
        return False
    return stat.S_ISREG(metadata.st_mode) and metadata.st_uid == os.getuid() and metadata.st_nlink == 1


def migrate_legacy_install(prefix):
    legacy_binary = prefix / "bin/alacritty"
    legacy_launcher = prefix / "share/applications/Alacritty.desktop"
    legacy_receipt = prefix / "share/doc/alacritty-workspace/VERSION"
    if not all(owned_file(path) for path in (legacy_binary, legacy_launcher, legacy_receipt)):
        return
    old_version = legacy_receipt.read_text().strip()
    identity = re.fullmatch(r"([0-9]+\.[0-9]+\.[0-9]+)-workspace\.([0-9a-f]{8})\.([0-9a-f]{12})", old_version)
    if identity is None:
        return
    launcher_data = legacy_launcher.read_bytes()
    config = configparser.ConfigParser(interpolation=None)
    config.optionxform = str
    try:
        config.read_string(launcher_data.decode())
    except (UnicodeError, configparser.Error):
        return
    expected_entry = {
        "Type": "Application", "Name": "Alacritty Workspace", "Icon": "Alacritty",
        "TryExec": str(legacy_binary).replace("\\", "\\\\"),
        "Exec": desktop_command(str(legacy_binary)), "Terminal": "false",
        "Categories": "System;TerminalEmulator;", "GenericName": "Terminal",
        "Comment": "A fast, cross-platform, OpenGL terminal emulator",
        "StartupNotify": "true", "StartupWMClass": "Alacritty", "Actions": "New;",
    }
    expected_action = {"Name": "New Terminal", "Exec": desktop_command(str(legacy_binary))}
    if (set(config.sections()) != {"Desktop Entry", "Desktop Action New"}
            or dict(config["Desktop Entry"]) != expected_entry
            or dict(config["Desktop Action New"]) != expected_action):
        return
    try:
        previous = subprocess.run([str(legacy_binary), "--version"], stdin=subprocess.DEVNULL,
            capture_output=True, text=True, timeout=5)
    except (OSError, subprocess.SubprocessError):
        return
    versions = {f"alacritty {identity[1]}{suffix} ({identity[2]})" for suffix in ("", "-dev")}
    if previous.returncode or previous.stdout.strip() not in versions:
        return
    binary_data = legacy_binary.read_bytes()
    receipt_data = legacy_receipt.read_bytes()
    backup = prefix / "share/doc/termust/legacy-alacritty" / old_version
    for name, content, mode in (("alacritty", binary_data, 0o755),
                                ("Alacritty.desktop", launcher_data, 0o644),
                                ("VERSION", receipt_data, 0o644)):
        destination = backup / name
        if destination.exists() or destination.is_symlink():
            if not owned_file(destination) or destination.read_bytes() != content:
                raise OSError("existing legacy backup does not match the previous install")
        else:
            write_atomic(destination, content, mode)
    if legacy_binary.read_bytes() != binary_data or legacy_launcher.read_bytes() != launcher_data:
        raise OSError("previous install changed during migration")
    descriptor, temporary = tempfile.mkstemp(prefix=".termust-alacritty-", dir=legacy_binary.parent)
    os.close(descriptor)
    os.unlink(temporary)
    try:
        os.symlink("termust", temporary)
        os.replace(temporary, legacy_binary)
    finally:
        if os.path.lexists(temporary):
            os.unlink(temporary)
    legacy_launcher.unlink()
    record = {"old_version": old_version, "backup_directory": str(backup.relative_to(prefix)),
              "binary_sha256": hashlib.sha256(binary_data).hexdigest(),
              "launcher_sha256": hashlib.sha256(launcher_data).hexdigest()}
    write_atomic(prefix / "share/doc/termust/legacy-alacritty-migration.json",
                 (json.dumps(record, indent=2) + "\n").encode())
    print("Migrated the previous Alacritty Workspace command and launcher to Termust.")

try:
    main()
except (RuntimeError, OSError, ValueError, KeyError, tarfile.TarError, subprocess.SubprocessError) as error:
    print("Error: " + str(error), file=sys.stderr)
    sys.exit(1)
TERMUST_PYTHON
}

main "$@"
