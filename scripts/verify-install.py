import argparse
import configparser
import hashlib
import json
import shlex
import shutil
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def desktop_argv(value):
    values = shlex.split(value.replace("\\\\", "\\"))
    return [value.replace("%%", "%") for value in values]


def main():
    parser = argparse.ArgumentParser(description="Verify the installed Termust binary and desktop entry.")
    parser.add_argument("--prefix", type=Path, default=Path.home() / ".local")
    args = parser.parse_args()
    prefix = args.prefix.expanduser().absolute()
    binary = prefix / "bin/termust"
    manifest = json.loads((ROOT / "dist/release.json").read_text())
    if not binary.is_file() or hashlib.sha256(binary.read_bytes()).hexdigest() != manifest["binary_sha256"]:
        raise SystemExit("Installed executable does not match the packaged release.")
    version = subprocess.check_output([str(binary), "--version"], text=True).strip()
    if version != manifest["binary_version"]:
        raise SystemExit("Installed executable version does not match the packaged release.")
    receipt = prefix / "share/doc/termust/VERSION"
    if receipt.read_text().strip() != manifest["version"]:
        raise SystemExit("Installed package version does not match the release.")
    installed = json.loads((prefix / "share/doc/termust/install.json").read_text())
    if installed.get("version") != manifest["version"] or installed.get("binary_sha256") != manifest["binary_sha256"]:
        raise SystemExit("Installed receipt does not match the packaged release.")
    subprocess.run(["bash", str(ROOT / "install.sh"), "--prefix", str(prefix), "--check"], check=True)
    launcher = prefix / "share/applications/Termust.desktop"
    config = configparser.ConfigParser(interpolation=None)
    config.optionxform = str
    config.read(launcher)
    entry = config["Desktop Entry"]
    if entry["Name"] != "Termust" or entry["TryExec"] != str(binary).replace("\\", "\\\\"):
        raise SystemExit("Desktop launcher does not point to the installed terminal.")
    if entry.get("Actions") != "new-window;" or config.has_section("Desktop Action New"):
        raise SystemExit("Desktop launcher has an unexpected or duplicate new-window action.")
    for section in ("Desktop Entry", "Desktop Action new-window"):
        if desktop_argv(config[section]["Exec"]) != [str(binary)]:
            raise SystemExit(f"Desktop command is incorrect in {section}.")
    if shutil.which("desktop-file-validate"):
        subprocess.run(["desktop-file-validate", str(launcher)], check=True)
    if not (prefix / "share/icons/hicolor/scalable/apps/Termust.svg").is_file():
        raise SystemExit("Installed desktop icon is missing.")
    if prefix == Path.home() / ".local" and Path(shutil.which("termust") or "/missing").resolve() != binary.resolve():
        raise SystemExit("The shell PATH does not select the newly installed terminal.")
    if shutil.which("infocmp"):
        for name in ("alacritty", "alacritty-direct"):
            if not any((Path.home() / ".terminfo" / directory / name).is_file()
                       for directory in (name[0], f"{ord(name[0]):x}")):
                raise SystemExit(f"User terminfo entry is missing: {name}")
            subprocess.run(["infocmp", name], check=True, stdout=subprocess.DEVNULL)
    migration = prefix / "share/doc/termust/legacy-alacritty-migration.json"
    if migration.exists():
        record = json.loads(migration.read_text())
        legacy_binary = prefix / "bin/alacritty"
        backup = prefix / "share/doc/termust/legacy-alacritty" / record["old_version"]
        if record["backup_directory"] != str(backup.relative_to(prefix)):
            raise SystemExit("Previous terminal backup location is invalid.")
        if not legacy_binary.is_symlink() or legacy_binary.resolve() != binary.resolve():
            raise SystemExit("The migrated Alacritty Workspace command does not select Termust.")
        if (prefix / "share/applications/Alacritty.desktop").exists():
            raise SystemExit("The previous managed launcher is still visible beside Termust.")
        for name, field in (("alacritty", "binary_sha256"), ("Alacritty.desktop", "launcher_sha256")):
            if hashlib.sha256((backup / name).read_bytes()).hexdigest() != record[field]:
                raise SystemExit(f"Previous terminal backup is incorrect: {name}")
        if (backup / "VERSION").read_text().strip() != record["old_version"]:
            raise SystemExit("Previous terminal backup version is incorrect.")
        print(f"Previous fork command: {legacy_binary} -> termust")
        print(f"Previous fork backup: {backup}")
    print(f"Installed package verified: {manifest['version']}")
    print(version)
    print(f"Shell executable: {binary}")
    print(f"Desktop launcher: {launcher}")
    if Path("/usr/bin/alacritty").exists():
        system_version = subprocess.check_output(["/usr/bin/alacritty", "--version"], text=True).strip()
        print(f"System package still available: {system_version}")


if __name__ == "__main__":
    main()
