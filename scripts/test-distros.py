import argparse
import configparser
import datetime
import hashlib
import json
import os
from pathlib import Path
import select
import shlex
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import uuid


ROOT = Path(__file__).resolve().parents[1]
DISTROS = {
    "ubuntu-22.04": "docker.io/library/ubuntu:22.04",
    "ubuntu-24.04": "docker.io/library/ubuntu:24.04",
    "debian-12": "docker.io/library/debian:12-slim",
    "debian-13": "docker.io/library/debian:13-slim",
    "fedora-43": "registry.fedoraproject.org/fedora:43",
    "fedora-44": "registry.fedoraproject.org/fedora:44",
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(command, **kwargs):
    result = subprocess.run(command, text=True, capture_output=True, timeout=120, **kwargs)
    if result.returncode:
        raise RuntimeError(f"Command failed ({result.returncode}): {shlex.join(command)}\n"
                           f"{result.stdout}\n{result.stderr}")
    return result.stdout.strip()


def stop(process):
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)


def inside():
    with tempfile.TemporaryDirectory(prefix="termust-distro-") as temporary:
        home = Path(temporary)
        os.environ.update(HOME=str(home), XDG_CONFIG_HOME=str(home / "config"),
                          XDG_DATA_HOME=str(home / ".local/share"), XDG_STATE_HOME=str(home / "state"),
                          XDG_CACHE_HOME=str(home / "cache"), XDG_RUNTIME_DIR=str(home / "runtime"),
                          PATH=str(home / ".local/bin") + ":" + os.environ["PATH"],
                          WINIT_UNIX_BACKEND="x11", LIBGL_ALWAYS_SOFTWARE="1", GALLIUM_DRIVER="llvmpipe")
        os.environ.pop("WAYLAND_DISPLAY", None)
        (home / "runtime").mkdir(mode=0o700)
        prefix = home / ".local"
        installer = ["bash", "/test/install.sh"]
        install = installer + ["--archive", "/test/termust-linux-x86_64.tar.gz"]
        outputs = [run(install)]
        binary = prefix / "bin/termust"
        with tarfile.open("/test/termust-linux-x86_64.tar.gz", "r:gz") as package:
            version = package.extractfile("termust/VERSION").read().decode().strip()
            expected_hash = hashlib.sha256(package.extractfile("termust/bin/termust").read()).hexdigest()
        assert digest(binary) == expected_hash, "Installed binary differs from release archive"
        assert Path(shutil.which("termust")).resolve() == binary, "PATH selects a different terminal"
        binary_version = run(["termust", "--version"])
        assert binary_version.startswith("termust "), binary_version
        assert (prefix / "share/doc/termust/VERSION").read_text().strip() == version
        receipt = json.loads((prefix / "share/doc/termust/install.json").read_text())
        outputs.append(run(installer + ["--check"]))
        outputs.append(run(install))
        assert digest(binary) == expected_hash, "Reinstallation changed executable"
        assert json.loads((prefix / "share/doc/termust/install.json").read_text()) == receipt, "Identical reinstall changed receipt"
        run(["infocmp", "alacritty"])
        run(["infocmp", "alacritty-direct"])
        launcher = prefix / "share/applications/Termust.desktop"
        run(["desktop-file-validate", str(launcher)])
        entry = configparser.ConfigParser(interpolation=None)
        entry.read(launcher)
        assert entry["Desktop Entry"]["Name"] == "Termust"
        assert entry["Desktop Entry"]["TryExec"] == str(binary)
        assert entry["Desktop Entry"]["Actions"] == "new-window;"
        assert shlex.split(entry["Desktop Entry"]["Exec"]) == [str(binary)]
        config = home / "config/termust/termust.toml"
        config.parent.mkdir(parents=True)
        config.write_text("[window]\ndimensions = { columns = 80, lines = 24 }\n")
        marker = home / "pty-proof"
        shell_command = 'test -t 0 && test -t 1 && { printf "%s\\n" "$$"; stty size; } > ' + shlex.quote(str(marker))
        graphical_log = home / "graphical.log"
        with graphical_log.open("w") as log:
            xvfb = subprocess.Popen(["Xvfb", "-displayfd", "1", "-screen", "0", "800x600x24", "-nolisten", "tcp"],
                                    text=True, stdout=subprocess.PIPE, stderr=log)
            app = None
            try:
                if not select.select([xvfb.stdout], [], [], 10)[0]:
                    raise RuntimeError("Xvfb did not report its display")
                display_number = xvfb.stdout.readline().strip()
                if not display_number.isdigit():
                    raise RuntimeError("Xvfb did not start")
                os.environ["DISPLAY"] = ":" + display_number
                app = subprocess.Popen([str(binary), "--config-file", str(config), "-e", "/bin/sh", "-c", shell_command],
                                       stdout=log, stderr=log)
                if app.wait(timeout=30) != 0 or not marker.exists():
                    raise RuntimeError("Graphical terminal did not run the PTY command")
                probe = marker.read_text().splitlines()
                assert len(probe) == 2 and int(probe[0]) > 0
                rows, columns = map(int, probe[1].split())
                assert rows > 0 and columns > 0, "PTY has invalid dimensions"
                marker.unlink()
                config.write_text(config.read_text() + "\n[terminal.shell]\nprogram = \"/bin/sh\"\nargs = "
                                  + json.dumps(["-c", shell_command]) + "\n")
                run(["dbus-run-session", "--", "gio", "launch", str(launcher)])
                deadline = time.monotonic() + 20
                while not marker.exists() and time.monotonic() < deadline:
                    time.sleep(0.05)
                if not marker.exists():
                    raise RuntimeError("Desktop launcher did not run the PTY command")
            except Exception as error:
                log.flush()
                raise RuntimeError(f"{error}\n{graphical_log.read_text()}") from error
            finally:
                if app is not None:
                    stop(app)
                stop(xvfb)
        release = {}
        for line in Path("/etc/os-release").read_text().splitlines():
            if "=" in line:
                key, value = line.split("=", 1)
                release[key] = value.strip('"')
        print(json.dumps({"status": "passed", "os": release.get("PRETTY_NAME"),
                          "glibc": run(["getconf", "GNU_LIBC_VERSION"]), "version": version,
                          "binary_version": binary_version, "binary_sha256": expected_hash,
                          "checks": ["install", "version", "receipt", "check", "reinstall", "path",
                                     "desktop-entry", "terminfo", "graphical-pty", "desktop-launch-pty"],
                          "installer_output": outputs}))


def prepare(distro, log_path):
    tag = "termust-distro-test:" + distro
    with tempfile.TemporaryDirectory(prefix="termust-distro-image-") as directory:
        context = Path(directory)
        shutil.copyfile(ROOT / "extra/linux/Dockerfile.test-distros", context / "Dockerfile")
        with log_path.open("w") as log:
            result = subprocess.run(["devrig", "build", str(context), "--platform", "linux/amd64",
                                     "--build-arg", "BASE_IMAGE=" + DISTROS[distro], "-t", tag],
                                    text=True, stdout=subprocess.PIPE, stderr=log, timeout=1800)
        if result.returncode:
            raise RuntimeError(f"Image build failed for {distro}; see {log_path}\n{log_path.read_text()[-6000:]}")
        metadata = json.loads(result.stdout)
        engine = metadata.get("tool", "docker")
        if engine not in ("docker", "podman"):
            raise RuntimeError(f"Unexpected container engine in devrig result: {engine}")
        image = json.loads(run([engine, "image", "inspect", tag]))[0]
        return engine, tag, image["Id"]


def main():
    parser = argparse.ArgumentParser(description="Verify the real Termust release on isolated Linux distributions.")
    parser.add_argument("--distro", action="append", choices=tuple(DISTROS))
    parser.add_argument("--archive", type=Path, default=ROOT / "dist/termust-linux-x86_64.tar.gz")
    parser.add_argument("--installer", type=Path, default=ROOT / "install.sh")
    parser.add_argument("--output", type=Path, default=ROOT / "dist/compatibility.json")
    parser.add_argument("--prepare-only", action="store_true")
    parser.add_argument("--inside", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.inside:
        inside()
        return
    if not shutil.which("devrig"):
        raise SystemExit("devrig is required to build disposable distro test images.")
    distros = list(dict.fromkeys(args.distro or DISTROS))
    output = args.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    logs = output.parent / "distro-test-logs"
    logs.mkdir(exist_ok=True)
    archive = args.archive.resolve()
    installer = args.installer.resolve()
    checksum = Path(str(archive) + ".sha256")
    if not args.prepare_only and not all(path.is_file() for path in (archive, installer, checksum)):
        raise SystemExit("Release archive, SHA256 file and installer are required. Build the portable package first.")
    with tempfile.TemporaryDirectory(prefix="termust-distro-input-") as temporary:
        captured = Path(temporary)
        runner = captured / "runner.py"
        shutil.copyfile(Path(__file__).resolve(), runner)
        if not args.prepare_only:
            for source, name in ((archive, "termust-linux-x86_64.tar.gz"),
                                 (checksum, "termust-linux-x86_64.tar.gz.sha256"), (installer, "install.sh")):
                shutil.copyfile(source, captured / name)
            archive = captured / "termust-linux-x86_64.tar.gz"
            checksum = captured / "termust-linux-x86_64.tar.gz.sha256"
            installer = captured / "install.sh"
        report = {"schema": 1, "tested_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  "archive_sha256": digest(archive) if archive.is_file() else None,
                  "installer_sha256": digest(installer), "test_runner_sha256": digest(runner),
                  "runtime_dockerfile_sha256": digest(ROOT / "extra/linux/Dockerfile.test-distros"),
                  "scope": "Linux userland with X11/Xvfb and Mesa software rendering", "results": []}
        for distro in distros:
            print(f"Preparing {distro}...", flush=True)
            try:
                engine, tag, image_id = prepare(distro, logs / (distro + "-build.log"))
                if args.prepare_only:
                    print(f"Ready: {distro} ({image_id})", flush=True)
                    continue
                name = "termust-distro-" + uuid.uuid4().hex[:12]
                command = [engine, "run", "--rm", "--name", name, "--platform", "linux/amd64",
                           "--network", "none", "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
                           "--user", "1000:1000", "--tmpfs", "/tmp:rw,exec,mode=1777", "--read-only"]
                for source, destination in ((archive, "/test/termust-linux-x86_64.tar.gz"),
                                            (checksum, "/test/termust-linux-x86_64.tar.gz.sha256"),
                                            (installer, "/test/install.sh"),
                                            (runner, "/test/runner.py")):
                    command += ["--mount", f"type=bind,src={source},dst={destination},readonly"]
                command += [tag, "python3", "-B", "/test/runner.py", "--inside"]
                try:
                    result = run(command)
                    (logs / (distro + "-run.log")).write_text(result + "\n")
                    record = json.loads(result.splitlines()[-1])
                    record.update(distro=distro, image=DISTROS[distro], image_id=image_id)
                    report["results"].append(record)
                    print(f"Passed: {distro}; {record['glibc']}; graphical PTY and desktop launch", flush=True)
                finally:
                    subprocess.run([engine, "rm", "-f", name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=30)
            except (RuntimeError, OSError, ValueError, subprocess.SubprocessError) as error:
                report["results"].append({"distro": distro, "status": "failed", "error": str(error)})
                print(f"Failed: {distro}: {error}", file=sys.stderr, flush=True)
            if not args.prepare_only:
                output.write_text(json.dumps(report, indent=2) + "\n")
    if any(result["status"] == "failed" for result in report["results"]):
        raise SystemExit(1)
    if not args.prepare_only:
        print(f"Verified {len(report['results'])} distributions. Report: {output}")


if __name__ == "__main__":
    main()
