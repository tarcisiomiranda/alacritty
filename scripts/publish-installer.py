import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import re


ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("publisher", ROOT / "scripts/publish-b2.py")
publisher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publisher)


def main():
    config = publisher.settings()
    installer = ROOT / "install.sh"
    data = installer.read_bytes()
    sha = hashlib.sha256(data).hexdigest()
    manifest = json.loads(publisher.download(config, "latest-macos-arm64.json"))
    mac_sha = manifest.get("sha256", "")
    if (manifest.get("schema") != 1 or manifest.get("platform") != "macos-arm64"
            or not isinstance(mac_sha, str) or not re.fullmatch(r"[0-9a-f]{64}", mac_sha)
            or manifest.get("archive") != f"macos/binary/{mac_sha}.tar.gz"):
        raise RuntimeError("macOS release manifest is invalid; installer was not changed")
    if hashlib.sha256(publisher.download(config, manifest["archive"])).hexdigest() != mac_sha:
        raise RuntimeError("macOS archive checksum mismatch; installer was not changed")
    with tempfile.TemporaryDirectory(prefix="termust-publish-installer-") as temporary:
        checksum = Path(temporary) / "install.sh.sha256"
        checksum.write_text(sha + "  install.sh\n")
        for target in [config, *publisher.mirror_targets(config)]:
            publisher.put(target, checksum, "install.sh.sha256", "text/plain; charset=utf-8")
            publisher.put(target, installer, "install.sh", "text/x-shellscript; charset=utf-8")
            if target == config:
                publisher.put(target, installer, "install-macos.sh", "text/x-shellscript; charset=utf-8")
            if publisher.download(target, "install.sh") != data:
                raise RuntimeError(f"Public installer differs from the local file in {target['bucket']}.")
            if publisher.download(target, "install.sh.sha256").decode().split() != [sha, "install.sh"]:
                raise RuntimeError(f"Public installer checksum mismatch in {target['bucket']}.")
            print(f"Common installer verified: {target['base_url']}/install.sh")


if __name__ == "__main__":
    main()
