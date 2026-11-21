import hashlib
import importlib.util
from pathlib import Path
import re


ROOT = Path(__file__).resolve().parents[1]
DIST = ROOT / "dist/macos-source"
spec = importlib.util.spec_from_file_location("publisher", ROOT / "scripts/publish-b2.py")
publisher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publisher)


def main():
    archive = DIST / "source.tar.gz"
    builder = DIST / "build-macos-release.sh"
    digest = (DIST / "source.sha256").read_text().strip()
    template = (ROOT / "scripts/build-macos-release.sh").read_text()
    if not re.fullmatch(r"[0-9a-f]{64}", digest) or hashlib.sha256(archive.read_bytes()).hexdigest() != digest:
        raise RuntimeError("macOS source archive checksum mismatch")
    if template.count("@SOURCE_SHA256@") != 1 or builder.read_text() != template.replace("@SOURCE_SHA256@", digest):
        raise RuntimeError("macOS builder does not match the source archive")
    config = publisher.settings()
    source_name = f"macos/source/{digest}.tar.gz"
    publisher.put(config, archive, source_name, "application/gzip", immutable=True)
    publisher.put(config, builder, "build-macos-release.sh", "text/x-shellscript; charset=utf-8")
    if hashlib.sha256(publisher.download(config, source_name)).hexdigest() != digest:
        raise RuntimeError("public macOS source archive checksum mismatch")
    if publisher.download(config, "build-macos-release.sh") != builder.read_bytes():
        raise RuntimeError("public macOS builder does not match the local file")
    print(f"macOS builder verified: {config['base_url']}/build-macos-release.sh")


if __name__ == "__main__":
    main()
