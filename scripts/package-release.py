import argparse
import gzip
import hashlib
import json
import platform
import re
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DIST = ROOT / "dist"
ARCHIVE = "termust-linux-x86_64.tar.gz"

def run(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()

def main():
    parser = argparse.ArgumentParser(description="Package a native or portable Termust binary.")
    parser.add_argument("--portable", action="store_true")
    args = parser.parse_args()
    if platform.system() != "Linux" or platform.machine() not in ("x86_64", "amd64"):
        raise SystemExit("This release currently supports Linux x86_64 only.")
    provenance = {}
    if args.portable:
        binary = DIST / "portable/termust"
        provenance = json.loads((DIST / "portable/build.json").read_text())
        if hashlib.sha256(binary.read_bytes()).hexdigest() != provenance["binary_sha256"]:
            raise SystemExit("Portable binary does not match its build metadata.")
    else:
        metadata = json.loads(run("cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"))
        binary = Path(metadata["target_directory"]) / "release" / "termust"
    if not binary.is_file():
        raise SystemExit("Release binary missing. Run mise run build:release first.")
    if not shutil.which("strip"):
        raise SystemExit("The strip command from binutils is required to package the release.")
    package_version = tomllib.loads((ROOT / "alacritty/Cargo.toml").read_text())["package"]["version"]
    commit = run("git", "rev-parse", "HEAD")
    epoch = int(run("git", "show", "-s", "--format=%ct", "HEAD"))
    DIST.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="termust-package-") as temporary:
        staging = Path(temporary) / "termust"
        binary_target = staging / "bin/termust"
        binary_target.parent.mkdir(parents=True)
        shutil.copyfile(binary, binary_target)
        binary_target.chmod(0o755)
        subprocess.run(["strip", "--strip-unneeded", str(binary_target)], check=True)
        binary_version = run(str(binary_target), "--version")
        assets = {
            "extra/linux/Termust.desktop": "share/applications/Termust.desktop",
            "extra/logo/termust.svg": "share/icons/hicolor/scalable/apps/Termust.svg",
            "extra/alacritty.info": "share/terminfo/alacritty.info",
            "extra/completions/termust.bash": "share/bash-completion/completions/termust",
            "extra/completions/termust.fish": "share/fish/vendor_completions.d/termust.fish",
            "extra/completions/_termust": "share/zsh/site-functions/_termust",
            "LICENSE-APACHE": "share/doc/termust/LICENSE-APACHE",
            "README.md": "share/doc/termust/README.md",
            "docs/tabs.md": "share/doc/termust/tabs.md",
            "docs/panes.md": "share/doc/termust/panes.md",
            "docs/productivity.md": "share/doc/termust/productivity.md",
            "docs/install.md": "share/doc/termust/install.md",
            "install.sh": "share/doc/termust/install.sh",
        }
        for source, target in assets.items():
            destination = staging / target
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / source, destination)
            destination.chmod(0o644)
        digest = hashlib.sha256()
        for path in sorted(staging.rglob("*")):
            if path.is_file():
                digest.update(str(path.relative_to(staging)).encode())
                digest.update(b"\0")
                digest.update(path.read_bytes())
        requirements = run("readelf", "--version-info", str(binary_target))
        glibc_versions = set(re.findall(r"GLIBC_(\d+(?:\.\d+)+)", requirements))
        minimum_glibc = max(glibc_versions, key=lambda value: tuple(map(int, value.split(".")))) if glibc_versions else None
        if args.portable and (minimum_glibc != provenance["minimum_glibc"] or tuple(map(int, minimum_glibc.split("."))) > (2, 31)):
            raise SystemExit("Portable glibc baseline does not match the build metadata.")
        inputs = [ROOT / source for source in assets]
        inputs += [ROOT / "Cargo.toml", ROOT / "Cargo.lock", ROOT / "install.sh", ROOT / "scripts/package-release.py"]
        for crate in ("alacritty", "alacritty_terminal", "alacritty_config", "alacritty_config_derive"):
            inputs += list((ROOT / crate).rglob("*.rs")) + [ROOT / crate / "Cargo.toml"]
        build_timestamp = int(max(path.stat().st_mtime for path in inputs))
        manifest = {
            "platform": "linux-x86_64",
            "binary_version": binary_version,
            "binary_sha256": hashlib.sha256(binary_target.read_bytes()).hexdigest(),
            "source_commit": commit,
            "source_dirty": bool(run("git", "status", "--porcelain")),
            "source_commit_time": epoch,
            "source_url": f"https://github.com/tarcisiomiranda/alacritty/tree/{commit}",
            "build_os": provenance.get("build_os", platform.freedesktop_os_release().get("PRETTY_NAME", "Linux")),
            "package_version": package_version,
            "build_timestamp": build_timestamp,
            "portable": args.portable,
            "build_provenance": provenance,
            "minimum_glibc": minimum_glibc,
            "installer_sha256": hashlib.sha256((ROOT / "install.sh").read_bytes()).hexdigest(),
            "archive": ARCHIVE,
        }
        digest.update(json.dumps(manifest, sort_keys=True).encode())
        version = f"{package_version.removesuffix('-dev')}-termust.{commit[:8]}.{digest.hexdigest()[:12]}"
        manifest["version"] = version
        (staging / "VERSION").write_text(version + "\n")
        (staging / "release.json").write_text(json.dumps(manifest, indent=2) + "\n")
        archive = DIST / ARCHIVE
        scratch = archive.with_suffix(".tmp")
        with scratch.open("wb") as raw:
            with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=epoch) as compressed:
                with tarfile.open(mode="w", fileobj=compressed, format=tarfile.PAX_FORMAT) as tar:
                    for path in [staging, *sorted(staging.rglob("*"))]:
                        info = tar.gettarinfo(str(path), arcname=str(path.relative_to(staging.parent)))
                        info.uid = info.gid = 0
                        info.uname = info.gname = "root"
                        info.mtime = epoch
                        info.mode = 0o755 if path.is_dir() or path == binary_target else 0o644
                        if path.is_file():
                            with path.open("rb") as data:
                                tar.addfile(info, data)
                        else:
                            tar.addfile(info)
        scratch.replace(archive)
        manifest["sha256"] = hashlib.sha256(archive.read_bytes()).hexdigest()
        manifest["size"] = archive.stat().st_size
        (DIST / (ARCHIVE + ".sha256")).write_text(f"{manifest['sha256']}  {ARCHIVE}\n")
        (DIST / "release.json").write_text(json.dumps(manifest, indent=2) + "\n")
        (DIST / "latest-linux-x86_64.txt").write_text(version + "\n")
        print(f"Packaged {binary_version}")
        print(f"Version: {version}")
        print(f"Archive: {archive} ({archive.stat().st_size:,} bytes)")
        print(f"SHA256: {manifest['sha256']}")
        print(f"Minimum glibc: {minimum_glibc}")

if __name__ == "__main__":
    main()
