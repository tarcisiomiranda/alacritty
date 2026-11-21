import gzip
import hashlib
from pathlib import Path
import tarfile


ROOT = Path(__file__).resolve().parents[1]
DIST = ROOT / "dist/macos-source"
CRATES = ("alacritty", "alacritty_terminal", "alacritty_config", "alacritty_config_derive")


def files():
    paths = [ROOT / name for name in ("Cargo.toml", "Cargo.lock", "README.md", "LICENSE-APACHE", "LICENSE-MIT", "install.sh")]
    for crate in CRATES:
        base = ROOT / crate
        paths.extend(base / name for name in ("Cargo.toml", "README.md") if (base / name).is_file() and not (base / name).is_symlink())
        paths.extend(path for path in (base / "src").rglob("*") if path.is_file() and not path.is_symlink())
        if (base / "build.rs").is_file():
            paths.append(base / "build.rs")
        if (base / "res").is_dir():
            paths.extend(path for path in (base / "res").rglob("*") if path.is_file() and not path.is_symlink())
    paths.extend(path for path in (ROOT / "extra/osx/Termust.app").rglob("*") if path.is_file() and not path.is_symlink())
    paths.append(ROOT / "extra/logo/termust.png")
    paths.extend(path for path in (ROOT / "extra/icons").rglob("*") if path.is_file() and not path.is_symlink())
    return sorted(set(paths))


def main():
    DIST.mkdir(parents=True, exist_ok=True)
    archive = DIST / "source.tar.gz"
    with archive.open("wb") as raw, gzip.GzipFile(fileobj=raw, mode="wb", mtime=0, filename="") as compressed, tarfile.open(fileobj=compressed, mode="w") as tar:
        for path in files():
            if not path.is_file() or path.is_symlink():
                raise RuntimeError(f"missing or unsafe source file: {path}")
            info = tar.gettarinfo(str(path), arcname="termust-source/" + path.relative_to(ROOT).as_posix())
            info.uid = info.gid = 0
            info.uname = info.gname = ""
            info.mtime = 0
            with path.open("rb") as source:
                tar.addfile(info, source)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    builder = (ROOT / "scripts/build-macos-release.sh").read_text()
    if builder.count("@SOURCE_SHA256@") != 1:
        raise RuntimeError("builder source hash placeholder is invalid")
    (DIST / "build-macos-release.sh").write_text(builder.replace("@SOURCE_SHA256@", digest))
    (DIST / "source.sha256").write_text(digest + "\n")
    print(f"macOS source SHA256: {digest}")
    print(f"Builder: {DIST / 'build-macos-release.sh'}")


if __name__ == "__main__":
    main()
